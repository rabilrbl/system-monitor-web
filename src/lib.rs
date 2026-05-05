pub mod model;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::extract::State;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, EXPIRES, PRAGMA};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use regex::Regex;
use tower_http::cors::CorsLayer;

use crate::model::{
    BatteryStats, CoreUsage, GpuInfo, MemInfo, MemSwap, NetworkStats, PowerStats, RefreshPayload,
    TemperatureSensor, TopBandwidthProcess, TopCpuProcess, TopMemProcess, TopNetInterface,
};

pub const EMBEDDED_INDEX_HTML: &str = include_str!("../system-monitor.html");

#[derive(Debug, Clone, PartialEq)]
pub struct GpuActivityMetric {
    pub utilization: u32,
    pub source: &'static str,
    pub detail: &'static str,
}

#[derive(Default)]
pub struct AppContext {
    telemetry: Mutex<TelemetryState>,
}

#[derive(Default)]
struct TelemetryState {
    cpu_prev: Option<CpuTimes>,
    core_prev: HashMap<String, CpuTimes>,
    net_prev: Option<NetTotalsSample>,
    net_top_prev: Option<NetTopSample>,
    intel_gpu_prev: HashMap<String, IntelGpuSample>,
    rapl_prev: Option<RaplSample>,
    bandwidth_prev: Option<BandwidthSample>,
}

#[derive(Clone, Copy)]
struct CpuTimes {
    user: u64,
    nice: u64,
    system: u64,
    idle: u64,
    iowait: u64,
    irq: u64,
    softirq: u64,
}

impl CpuTimes {
    fn active(self) -> u64 {
        self.user
            .saturating_add(self.nice)
            .saturating_add(self.system)
            .saturating_add(self.irq)
            .saturating_add(self.softirq)
    }

    fn total(self) -> u64 {
        self.active()
            .saturating_add(self.idle)
            .saturating_add(self.iowait)
    }
}

struct NetTotalsSample {
    rx: u64,
    tx: u64,
    at: Instant,
}

struct NetTopSample {
    at: Instant,
    samples: HashMap<String, (u64, u64)>,
}

struct IntelGpuSample {
    rc6: Option<f64>,
    at: Instant,
}

struct RaplSample {
    pkg: u64,
    core: u64,
    uncore: u64,
    total: u64,
    at: Instant,
}

struct BandwidthSample {
    at: Instant,
    samples: HashMap<String, ProcessBandwidthCounter>,
}

#[derive(Clone)]
struct ProcessBandwidthCounter {
    sent: u64,
    recv: u64,
    pid: i32,
    connections: u32,
    name: String,
    command: String,
}

fn clamp_percent(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    value.clamp(0.0, 100.0)
}

pub fn derive_gpu_activity_metric(
    elapsed_ms: f64,
    rc6_delta_ms: Option<f64>,
    min_freq: f64,
    max_freq: f64,
    act_freq: f64,
) -> GpuActivityMetric {
    if let Some(rc6_delta) = rc6_delta_ms {
        if elapsed_ms > 0.0 {
            let idle_ms = rc6_delta.clamp(0.0, elapsed_ms);
            let active_ms = (elapsed_ms - idle_ms).max(0.0);
            let utilization = ((active_ms * 100.0) / elapsed_ms).round();
            return GpuActivityMetric {
                utilization: clamp_percent(utilization) as u32,
                source: "rc6-residency",
                detail: "Actual active time from RC6 idle residency",
            };
        }
    }

    let freq_range = max_freq - min_freq;
    if freq_range > 0.0 && act_freq > 0.0 {
        let utilization = ((act_freq - min_freq) * 100.0) / freq_range;
        return GpuActivityMetric {
            utilization: clamp_percent(utilization).round() as u32,
            source: "frequency-fallback",
            detail: "Estimated from graphics clock range",
        };
    }

    GpuActivityMetric {
        utilization: 0,
        source: "unavailable",
        detail: "Usage telemetry unavailable",
    }
}

pub fn build_router(ctx: Arc<AppContext>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/index.html", get(index))
        .route("/refresh", get(refresh))
        .route("/api/system/stat", get(api_stat))
        .route("/api/system/meminfo", get(api_meminfo))
        .route("/api/system/memtotal", get(api_meminfo))
        .route("/api/system/hostname", get(api_hostname))
        .route("/api/system/cpu_model", get(api_cpu_model))
        .route("/api/system/gpu", get(api_gpu))
        .route("/api/system/intel_gpu", get(api_intel_gpu))
        .route("/api/system/network", get(api_network))
        .route("/api/system/temperatures", get(api_temperatures))
        .route("/api/system/top_cpu", get(api_top_cpu))
        .route("/api/system/top_mem", get(api_top_mem))
        .route("/api/system/top_net", get(api_top_net))
        .route("/api/system/top_bandwidth", get(api_top_bandwidth))
        .route("/api/system/uptime", get(api_uptime))
        .route("/api/system/loadavg", get(api_loadavg))
        .route("/api/system/power", get(api_power))
        .route("/api/system/battery", get(api_battery))
        .route("/api/system/cores", get(api_cores))
        .fallback(not_found)
        .layer(CorsLayer::permissive())
        .with_state(ctx)
}

async fn index() -> impl IntoResponse {
    Html(EMBEDDED_INDEX_HTML)
}

async fn not_found() -> Response {
    text_response(StatusCode::NOT_FOUND, "Not found".to_string())
}

async fn api_stat() -> Response {
    text_response(StatusCode::OK, read_text_file("/proc/stat"))
}

async fn api_meminfo() -> Response {
    text_response(StatusCode::OK, read_text_file("/proc/meminfo"))
}

async fn api_hostname() -> Response {
    text_response(StatusCode::OK, get_hostname())
}

async fn api_cpu_model() -> Response {
    text_response(StatusCode::OK, get_cpu_model())
}

async fn api_gpu(State(ctx): State<Arc<AppContext>>) -> Response {
    json_response(StatusCode::OK, get_gpu(&ctx))
}

async fn api_intel_gpu(State(ctx): State<Arc<AppContext>>) -> Response {
    json_response(StatusCode::OK, get_intel_gpu(&ctx))
}

async fn api_network(State(ctx): State<Arc<AppContext>>) -> Response {
    json_response(StatusCode::OK, get_network(&ctx))
}

async fn api_temperatures() -> Response {
    json_response(StatusCode::OK, get_temperatures())
}

async fn api_top_cpu() -> Response {
    json_response(StatusCode::OK, get_top_cpu())
}

async fn api_top_mem() -> Response {
    json_response(StatusCode::OK, get_top_mem())
}

async fn api_top_net(State(ctx): State<Arc<AppContext>>) -> Response {
    json_response(StatusCode::OK, get_top_net(&ctx))
}

async fn api_top_bandwidth(State(ctx): State<Arc<AppContext>>) -> Response {
    json_response(StatusCode::OK, get_top_bandwidth_processes(&ctx))
}

async fn api_uptime() -> Response {
    text_response(StatusCode::OK, get_uptime())
}

async fn api_loadavg() -> Response {
    text_response(StatusCode::OK, get_loadavg())
}

async fn api_power(State(ctx): State<Arc<AppContext>>) -> Response {
    json_response(StatusCode::OK, get_power(&ctx))
}

async fn api_battery() -> Response {
    json_response(StatusCode::OK, get_battery())
}

async fn api_cores(State(ctx): State<Arc<AppContext>>) -> Response {
    let cores = get_cores(&ctx);
    let body = cores
        .iter()
        .map(|core| format!("{:.1}\t{}", core.pct, core.label))
        .collect::<Vec<_>>()
        .join("\n");
    text_response(StatusCode::OK, body)
}

async fn refresh(State(ctx): State<Arc<AppContext>>) -> Response {
    let payload = RefreshPayload {
        cpu: get_cpu_usage(&ctx),
        cpu_model: get_cpu_model(),
        cores: get_cores(&ctx),
        mem: get_mem_info(),
        gpu: get_gpu(&ctx),
        network: get_network(&ctx),
        temperatures: get_temperatures(),
        top_cpu: get_top_cpu(),
        top_mem: get_top_mem(),
        top_net: get_top_net(&ctx),
        uptime: get_uptime(),
        loadavg: get_loadavg(),
    };

    json_response(StatusCode::OK, payload)
}

fn text_response(status: StatusCode, body: String) -> Response {
    let mut response = body.into_response();
    *response.status_mut() = status;
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    add_cache_headers(response)
}

fn json_response<T: serde::Serialize>(status: StatusCode, value: T) -> Response {
    let mut response = Json(value).into_response();
    *response.status_mut() = status;
    add_cache_headers(response)
}

fn add_cache_headers(mut response: Response) -> Response {
    response.headers_mut().insert(
        CACHE_CONTROL,
        HeaderValue::from_static("no-store, no-cache, must-revalidate, proxy-revalidate"),
    );
    response
        .headers_mut()
        .insert(PRAGMA, HeaderValue::from_static("no-cache"));
    response
        .headers_mut()
        .insert(EXPIRES, HeaderValue::from_static("0"));
    response
}

fn read_text_file(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

fn get_hostname() -> String {
    fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::env::var("HOSTNAME")
                .ok()
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| "localhost".to_string())
}

fn read_number(path: &str) -> Option<f64> {
    let value = read_text_file(path);
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed.parse::<f64>().ok()
}

fn read_i64(path: &str) -> i64 {
    read_text_file(path).trim().parse::<i64>().unwrap_or(0)
}

fn first_existing_path(paths: &[&str]) -> Option<String> {
    paths
        .iter()
        .find(|path| Path::new(path).exists())
        .map(|path| (*path).to_string())
}

fn parse_cpu_total_from_stat(content: &str) -> Option<CpuTimes> {
    for line in content.lines() {
        if !line.starts_with("cpu ") {
            continue;
        }
        let mut parts = line.split_whitespace().skip(1);
        return Some(CpuTimes {
            user: parts.next().and_then(|x| x.parse().ok()).unwrap_or(0),
            nice: parts.next().and_then(|x| x.parse().ok()).unwrap_or(0),
            system: parts.next().and_then(|x| x.parse().ok()).unwrap_or(0),
            idle: parts.next().and_then(|x| x.parse().ok()).unwrap_or(0),
            iowait: parts.next().and_then(|x| x.parse().ok()).unwrap_or(0),
            irq: parts.next().and_then(|x| x.parse().ok()).unwrap_or(0),
            softirq: parts.next().and_then(|x| x.parse().ok()).unwrap_or(0),
        });
    }
    None
}

fn parse_core_times_from_stat(content: &str) -> HashMap<String, CpuTimes> {
    let mut map = HashMap::new();
    for line in content.lines() {
        let mut pieces = line.split_whitespace();
        let Some(label) = pieces.next() else {
            continue;
        };
        if !label.starts_with("cpu") || label == "cpu" {
            continue;
        }

        let values = pieces
            .take(7)
            .map(|v| v.parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>();
        if values.len() < 7 {
            continue;
        }

        map.insert(
            label.to_string(),
            CpuTimes {
                user: values[0],
                nice: values[1],
                system: values[2],
                idle: values[3],
                iowait: values[4],
                irq: values[5],
                softirq: values[6],
            },
        );
    }
    map
}

fn get_cpu_usage(ctx: &Arc<AppContext>) -> f64 {
    let content = read_text_file("/proc/stat");
    let Some(curr) = parse_cpu_total_from_stat(&content) else {
        return 0.0;
    };

    let mut telemetry = ctx.telemetry.lock().expect("telemetry mutex poisoned");
    let Some(prev) = telemetry.cpu_prev.replace(curr) else {
        return 0.0;
    };

    let delta_total = curr.total().saturating_sub(prev.total());
    if delta_total == 0 {
        return 0.0;
    }

    let delta_active = curr.active().saturating_sub(prev.active());
    (delta_active as f64) * 100.0 / (delta_total as f64)
}

fn get_cpu_model() -> String {
    read_text_file("/proc/cpuinfo")
        .lines()
        .find(|line| line.starts_with("model name"))
        .and_then(|line| line.split(':').nth(1))
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|| "Unknown".to_string())
}

fn get_cores(ctx: &Arc<AppContext>) -> Vec<CoreUsage> {
    let content = read_text_file("/proc/stat");
    let curr = parse_core_times_from_stat(&content);

    let mut telemetry = ctx.telemetry.lock().expect("telemetry mutex poisoned");
    if telemetry.core_prev.is_empty() {
        telemetry.core_prev = curr;
        return Vec::new();
    }

    let mut result = Vec::new();
    for (label, current) in &curr {
        let Some(previous) = telemetry.core_prev.get(label) else {
            continue;
        };

        let total_delta = current.total().saturating_sub(previous.total());
        if total_delta == 0 {
            continue;
        }

        let active_delta = current.active().saturating_sub(previous.active());
        result.push(CoreUsage {
            label: label.clone(),
            pct: (active_delta as f64) * 100.0 / (total_delta as f64),
        });
    }

    telemetry.core_prev = curr;
    result.sort_by(|a, b| a.label.cmp(&b.label));
    result
}

fn get_mem_info() -> MemInfo {
    let mut mem = HashMap::<String, u64>::new();
    for line in read_text_file("/proc/meminfo").lines() {
        let Some((key, raw_val)) = line.split_once(':') else {
            continue;
        };
        let value_kb = raw_val
            .split_whitespace()
            .next()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        mem.insert(key.trim().to_string(), value_kb.saturating_mul(1024));
    }

    let total = *mem.get("MemTotal").unwrap_or(&0);
    let avail = *mem
        .get("MemAvailable")
        .or_else(|| mem.get("MemFree"))
        .unwrap_or(&0);
    let used = total.saturating_sub(avail);

    let swap_total = *mem.get("SwapTotal").unwrap_or(&0);
    let swap_free = *mem.get("SwapFree").unwrap_or(&0);
    let swap_used = swap_total.saturating_sub(swap_free);

    MemInfo {
        total,
        avail,
        used,
        pct: if total > 0 {
            (used as f64) * 100.0 / (total as f64)
        } else {
            0.0
        },
        swap: MemSwap {
            total: swap_total,
            free: swap_free,
            used: swap_used,
            pct: if swap_total > 0 {
                (swap_used as f64) * 100.0 / (swap_total as f64)
            } else {
                0.0
            },
        },
    }
}

fn ps_output(sort_arg: &str) -> Option<String> {
    let output = Command::new("ps")
        .arg("aux")
        .arg(sort_arg)
        .arg("--no-headers")
        .arg("-ww")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

fn is_helper_process(name: &str) -> bool {
    Path::new(name)
        .file_name()
        .and_then(|part| part.to_str())
        .is_some_and(|base| matches!(base, "ps"))
}

fn get_top_cpu() -> Vec<TopCpuProcess> {
    let Some(output) = ps_output("--sort=-%cpu") else {
        return Vec::new();
    };

    let mut result = output
        .lines()
        .filter_map(|line| {
            let parts = line.split_whitespace().collect::<Vec<_>>();
            if parts.len() < 11 {
                return None;
            }
            let name = parts[10].to_string();
            if is_helper_process(&name) {
                return None;
            }
            let cpu = parts[2].parse::<f64>().unwrap_or(0.0);
            Some(TopCpuProcess { name, cpu })
        })
        .collect::<Vec<_>>();
    result.truncate(20);
    result
}

fn get_top_mem() -> Vec<TopMemProcess> {
    let Some(output) = ps_output("--sort=-%mem") else {
        return Vec::new();
    };

    let mut result = output
        .lines()
        .filter_map(|line| {
            let parts = line.split_whitespace().collect::<Vec<_>>();
            if parts.len() < 11 {
                return None;
            }
            let name = parts[10].to_string();
            if is_helper_process(&name) {
                return None;
            }
            let mem = parts[3].parse::<f64>().unwrap_or(0.0);
            Some(TopMemProcess { name, mem })
        })
        .collect::<Vec<_>>();
    result.truncate(20);
    result
}

fn parse_net_dev_totals(content: &str) -> (u64, u64) {
    let re = Regex::new(r"^\s*[^:]+:\s+(\d+)\s+\d+\s+\d+\s+\d+\s+\d+\s+\d+\s+\d+\s+\d+\s+(\d+)")
        .expect("valid net regex");

    let mut rx = 0u64;
    let mut tx = 0u64;
    for line in content.lines().skip(2) {
        if let Some(caps) = re.captures(line) {
            rx = rx.saturating_add(caps[1].parse::<u64>().unwrap_or(0));
            tx = tx.saturating_add(caps[2].parse::<u64>().unwrap_or(0));
        }
    }
    (rx, tx)
}

fn parse_net_dev_interfaces(content: &str) -> HashMap<String, (u64, u64)> {
    let re = Regex::new(r"^\s*([^:]+):\s+(\d+)\s+\d+\s+\d+\s+\d+\s+\d+\s+\d+\s+\d+\s+\d+\s+(\d+)")
        .expect("valid iface regex");

    let mut map = HashMap::new();
    for line in content.lines().skip(2) {
        if let Some(caps) = re.captures(line) {
            let iface = caps[1].trim().to_string();
            let rx = caps[2].parse::<u64>().unwrap_or(0);
            let tx = caps[3].parse::<u64>().unwrap_or(0);
            map.insert(iface, (rx, tx));
        }
    }
    map
}

fn get_network(ctx: &Arc<AppContext>) -> NetworkStats {
    let content = read_text_file("/proc/net/dev");
    let (rx, tx) = parse_net_dev_totals(&content);
    let now = Instant::now();

    let mut telemetry = ctx.telemetry.lock().expect("telemetry mutex poisoned");
    let Some(prev) = telemetry
        .net_prev
        .replace(NetTotalsSample { rx, tx, at: now })
    else {
        return NetworkStats {
            rx_bytes: 0.0,
            tx_bytes: 0.0,
        };
    };

    let dt = now.duration_since(prev.at).as_secs_f64();
    if dt <= 0.0 {
        return NetworkStats {
            rx_bytes: 0.0,
            tx_bytes: 0.0,
        };
    }

    let rx_rate = rx.saturating_sub(prev.rx) as f64 / dt;
    let tx_rate = tx.saturating_sub(prev.tx) as f64 / dt;

    NetworkStats {
        rx_bytes: rx_rate,
        tx_bytes: tx_rate,
    }
}

fn get_top_net(ctx: &Arc<AppContext>) -> Vec<TopNetInterface> {
    let content = read_text_file("/proc/net/dev");
    let current = parse_net_dev_interfaces(&content);
    let now = Instant::now();

    let mut telemetry = ctx.telemetry.lock().expect("telemetry mutex poisoned");
    let Some(prev) = telemetry.net_top_prev.replace(NetTopSample {
        at: now,
        samples: current.clone(),
    }) else {
        return Vec::new();
    };

    let dt = now.duration_since(prev.at).as_secs_f64().max(0.001);
    let mut result = Vec::new();

    for (name, (rx, tx)) in current {
        let Some((prev_rx, prev_tx)) = prev.samples.get(&name) else {
            continue;
        };

        let d_rx = rx.saturating_sub(*prev_rx);
        let d_tx = tx.saturating_sub(*prev_tx);

        let rx_mbps = (d_rx as f64) * 8.0 / 1_000_000.0 / dt;
        let tx_mbps = (d_tx as f64) * 8.0 / 1_000_000.0 / dt;
        let speed = (d_rx + d_tx) as f64 / dt;

        result.push(TopNetInterface {
            name,
            rx_mbps,
            tx_mbps,
            speed,
        });
    }

    result.sort_by(|a, b| {
        (b.rx_mbps + b.tx_mbps)
            .partial_cmp(&(a.rx_mbps + a.tx_mbps))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    result.truncate(20);
    result
}

fn get_temperatures() -> Vec<TemperatureSensor> {
    let mut sensors = Vec::new();
    let mut seen = HashSet::new();

    if let Ok(entries) = fs::read_dir("/sys/class/thermal") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("thermal_zone") {
                continue;
            }
            let base = entry.path();
            let temp = read_number(base.join("temp").to_string_lossy().as_ref());
            let Some(temp_value) = temp else {
                continue;
            };

            let label = read_text_file(base.join("type").to_string_lossy().as_ref())
                .trim()
                .to_string();
            let sensor = TemperatureSensor {
                kind: "thermal".to_string(),
                id: name.clone(),
                label: if label.is_empty() {
                    name.clone()
                } else {
                    label
                },
                source: format!("thermal/{name}"),
                value_c: temp_value / 1000.0,
            };
            let key = format!(
                "{}:{}:{}:{:.3}",
                sensor.kind, sensor.id, sensor.label, sensor.value_c
            );
            if seen.insert(key) {
                sensors.push(sensor);
            }
        }
    }

    let temp_re = Regex::new(r"^(temp\d+)_input$").expect("valid temp regex");
    if let Ok(entries) = fs::read_dir("/sys/class/hwmon") {
        for entry in entries.flatten() {
            let hwmon_name = entry.file_name().to_string_lossy().to_string();
            if !hwmon_name.starts_with("hwmon") {
                continue;
            }
            let base = entry.path();
            let chip = read_text_file(base.join("name").to_string_lossy().as_ref())
                .trim()
                .to_string();

            let Ok(files) = fs::read_dir(&base) else {
                continue;
            };
            for file in files.flatten() {
                let file_name = file.file_name().to_string_lossy().to_string();
                let Some(cap) = temp_re.captures(&file_name) else {
                    continue;
                };
                let idx = cap[1].to_string();
                let temp = read_number(base.join(&file_name).to_string_lossy().as_ref());
                let Some(temp_value) = temp else {
                    continue;
                };

                let default_label = format!(
                    "{} {}",
                    if chip.is_empty() { &hwmon_name } else { &chip },
                    idx
                );
                let label = {
                    let raw = read_text_file(
                        base.join(format!("{}_label", idx))
                            .to_string_lossy()
                            .as_ref(),
                    );
                    let trimmed = raw.trim();
                    if trimmed.is_empty() {
                        default_label
                    } else {
                        trimmed.to_string()
                    }
                };

                let sensor = TemperatureSensor {
                    kind: "hwmon".to_string(),
                    id: format!("{}:{}", hwmon_name, idx),
                    label,
                    source: format!(
                        "{}/{}",
                        if chip.is_empty() { &hwmon_name } else { &chip },
                        idx
                    ),
                    value_c: temp_value / 1000.0,
                };
                let key = format!(
                    "{}:{}:{}:{:.3}",
                    sensor.kind, sensor.id, sensor.label, sensor.value_c
                );
                if seen.insert(key) {
                    sensors.push(sensor);
                }
            }
        }
    }

    sensors.sort_by(|a, b| {
        b.value_c
            .partial_cmp(&a.value_c)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.label.cmp(&b.label))
    });
    sensors
}

fn get_uptime() -> String {
    read_text_file("/proc/uptime")
        .split_whitespace()
        .next()
        .unwrap_or("0")
        .to_string()
}

fn get_loadavg() -> String {
    read_text_file("/proc/loadavg").trim().to_string()
}

fn pick_battery_base() -> Option<String> {
    let preferred = "/sys/class/power_supply/BAT0";
    if Path::new(preferred).exists() {
        return Some(preferred.to_string());
    }

    let entries = fs::read_dir("/sys/class/power_supply").ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("BAT") {
            return Some(format!("/sys/class/power_supply/{name}"));
        }
    }
    None
}

fn pick_ac_base() -> Option<String> {
    for preferred in ["ADP1", "AC", "ACAD"] {
        let path = format!("/sys/class/power_supply/{preferred}");
        if Path::new(&path).exists() {
            return Some(path);
        }
    }

    let entries = fs::read_dir("/sys/class/power_supply").ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("ADP") || name.starts_with("AC") {
            return Some(format!("/sys/class/power_supply/{name}"));
        }
    }
    None
}

fn power_supply_watts_from_values(
    power_now: i64,
    voltage_now: i64,
    voltage_max: i64,
    voltage_min: i64,
    current_now: i64,
    current_max: i64,
    allow_derived_power: bool,
) -> Option<f64> {
    let direct_power = power_now.abs();
    if direct_power > 0 {
        return Some(direct_power as f64 / 1e6);
    }

    if !allow_derived_power {
        return None;
    }

    let voltage = [voltage_now, voltage_max, voltage_min]
        .into_iter()
        .find(|value| *value > 0)
        .unwrap_or(0);
    let current = [current_now, current_max]
        .into_iter()
        .find(|value| value.abs() > 0)
        .unwrap_or(0)
        .abs();

    if voltage > 0 && current > 0 {
        Some((voltage as f64) * (current as f64) / 1e12)
    } else {
        None
    }
}

fn read_power_supply_watts(base: &str) -> Option<f64> {
    power_supply_watts_from_values(
        read_i64(&format!("{base}/power_now")),
        read_i64(&format!("{base}/voltage_now")),
        read_i64(&format!("{base}/voltage_max")),
        read_i64(&format!("{base}/voltage_min")),
        read_i64(&format!("{base}/current_now")),
        read_i64(&format!("{base}/current_max")),
        true,
    )
}

fn read_power_supply_direct_watts(base: &str) -> Option<f64> {
    power_supply_watts_from_values(read_i64(&format!("{base}/power_now")), 0, 0, 0, 0, 0, false)
}

fn get_online_external_power_watts() -> Option<f64> {
    let entries = fs::read_dir("/sys/class/power_supply").ok()?;
    entries.flatten().find_map(|entry| {
        let base = entry.path();
        let base = base.to_string_lossy();
        let supply_type = read_text_file(&format!("{base}/type")).trim().to_string();
        if supply_type == "Battery" || read_i64(&format!("{base}/online")) != 1 {
            return None;
        }
        read_power_supply_direct_watts(&base)
    })
}

fn get_battery() -> BatteryStats {
    let mut result = BatteryStats {
        status: "Unknown".to_string(),
        capacity: 0,
        percentage: 0,
        voltage: 0,
        current: 0,
        power: 0.0,
        charge_full: 0,
        charge_now: 0,
        cycle_count: 0,
        ac_online: false,
        time_remaining: None,
        charge_full_design: 0,
        manufacturer: String::new(),
        model_name: String::new(),
    };

    let Some(base) = pick_battery_base() else {
        return result;
    };
    let ac_base = pick_ac_base();

    result.status = read_text_file(&format!("{base}/status")).trim().to_string();
    result.capacity = read_i64(&format!("{base}/capacity"));
    result.percentage = result.capacity;
    result.voltage = read_i64(&format!("{base}/voltage_now"));
    result.current = read_i64(&format!("{base}/current_now")).abs();
    result.charge_full = read_i64(&format!("{base}/charge_full"));
    result.charge_now = read_i64(&format!("{base}/charge_now"));
    result.cycle_count = read_i64(&format!("{base}/cycle_count"));
    result.charge_full_design = read_i64(&format!("{base}/charge_full_design"));
    result.manufacturer = read_text_file(&format!("{base}/manufacturer"))
        .trim()
        .to_string();
    result.model_name = read_text_file(&format!("{base}/model_name"))
        .trim()
        .to_string();

    if let Some(ac) = ac_base {
        result.ac_online = read_i64(&format!("{ac}/online")) == 1;
    }

    result.power = read_power_supply_watts(&base)
        .unwrap_or_else(|| (result.voltage as f64) * (result.current as f64) / 1e12);
    if result.power <= 0.05 && result.ac_online {
        if let Some(external_power) = get_online_external_power_watts() {
            result.power = external_power;
        }
    }

    if result.current > 1000 {
        if result.status == "Discharging" {
            result.time_remaining = Some((result.charge_now * 60 / result.current).max(0));
        } else if result.status == "Charging" {
            let rem = (result.charge_full - result.charge_now).max(0);
            result.time_remaining = Some((rem * 60 / result.current).max(0));
        }
    }

    result
}

fn get_power(ctx: &Arc<AppContext>) -> PowerStats {
    let mut result = PowerStats {
        package: 0.0,
        core: 0.0,
        uncore: 0.0,
        dram: 0.0,
        total: 0.0,
    };

    let pkg_path = first_existing_path(&[
        "/sys/class/powercap/intel-rapl:0/energy_uj",
        "/sys/class/powercap/intel-rapl-mmio:0/energy_uj",
    ]);
    let core_path = first_existing_path(&[
        "/sys/class/powercap/intel-rapl:0:0/energy_uj",
        "/sys/class/powercap/intel-rapl-mmio:0:0/energy_uj",
    ]);
    let uncore_path = first_existing_path(&[
        "/sys/class/powercap/intel-rapl:0:1/energy_uj",
        "/sys/class/powercap/intel-rapl-mmio:0:1/energy_uj",
    ]);
    let total_path = first_existing_path(&[
        "/sys/class/powercap/intel-rapl:1/energy_uj",
        "/sys/class/powercap/intel-rapl-mmio:1/energy_uj",
    ]);

    let pkg_energy = pkg_path.as_deref().map(read_i64).unwrap_or(0).max(0) as u64;
    let core_energy = core_path.as_deref().map(read_i64).unwrap_or(0).max(0) as u64;
    let uncore_energy = uncore_path.as_deref().map(read_i64).unwrap_or(0).max(0) as u64;
    let total_energy = total_path.as_deref().map(read_i64).unwrap_or(0).max(0) as u64;
    if pkg_energy == 0 && total_energy == 0 {
        return result;
    }

    let now = Instant::now();
    let mut telemetry = ctx.telemetry.lock().expect("telemetry mutex poisoned");
    let Some(prev) = telemetry.rapl_prev.replace(RaplSample {
        pkg: pkg_energy,
        core: core_energy,
        uncore: uncore_energy,
        total: total_energy,
        at: now,
    }) else {
        return result;
    };

    let dt = now.duration_since(prev.at).as_secs_f64();
    if dt <= 0.1 {
        return result;
    }

    let max_energy = 0xFFFF_FFFFu64;
    let delta = |current: u64, previous: u64| {
        if current >= previous {
            current - previous
        } else {
            max_energy.saturating_sub(previous).saturating_add(current)
        }
    };

    let pkg_delta = delta(pkg_energy, prev.pkg);
    let core_delta = delta(core_energy, prev.core);
    let uncore_delta = delta(uncore_energy, prev.uncore);
    let total_delta = delta(total_energy, prev.total);

    result.package = pkg_delta as f64 / dt / 1e6;
    result.core = core_delta as f64 / dt / 1e6;
    result.uncore = uncore_delta as f64 / dt / 1e6;
    result.total = if total_delta > 0 {
        total_delta as f64 / dt / 1e6
    } else {
        result.package
    };
    result
}

fn find_intel_gpu_card_path() -> Option<String> {
    let entries = fs::read_dir("/sys/class/drm").ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("card") || name.contains('-') {
            continue;
        }

        let base = format!("/sys/class/drm/{name}");
        let vendor = read_text_file(&format!("{base}/device/vendor"))
            .trim()
            .to_ascii_lowercase();
        let class_code = read_text_file(&format!("{base}/device/class"))
            .trim()
            .to_ascii_lowercase();
        if vendor == "0x8086" && class_code.starts_with("0x03") {
            return Some(base);
        }
    }
    None
}

fn get_intel_gpu(ctx: &Arc<AppContext>) -> Option<GpuInfo> {
    let card_path = find_intel_gpu_card_path()?;
    let gt_path = format!("{card_path}/gt/gt0");

    let cur_freq = read_number(&format!("{gt_path}/rps_cur_freq_mhz")).unwrap_or(0.0);
    let max_freq = read_number(&format!("{gt_path}/rps_max_freq_mhz")).unwrap_or(0.0);
    let min_freq = read_number(&format!("{gt_path}/rps_min_freq_mhz")).unwrap_or(0.0);
    let act_freq = read_number(&format!("{gt_path}/rps_act_freq_mhz")).unwrap_or(cur_freq);
    let rc6_now = read_number(&format!("{gt_path}/rc6_residency_ms"));

    let now = Instant::now();

    let mut telemetry = ctx.telemetry.lock().expect("telemetry mutex poisoned");
    let prev = telemetry.intel_gpu_prev.get(&card_path);

    let elapsed_ms = prev
        .map(|s| now.duration_since(s.at).as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    let rc6_delta_ms = match (prev.and_then(|p| p.rc6), rc6_now) {
        (Some(previous), Some(current)) => Some((current - previous).max(0.0)),
        _ => None,
    };

    let activity =
        derive_gpu_activity_metric(elapsed_ms, rc6_delta_ms, min_freq, max_freq, act_freq);

    telemetry.intel_gpu_prev.insert(
        card_path,
        IntelGpuSample {
            rc6: rc6_now,
            at: now,
        },
    );

    Some(GpuInfo {
        name: "Intel Iris Xe Graphics".to_string(),
        utilization: activity.utilization as f64,
        memory: 0.0,
        mem_used: 0,
        mem_total: 0,
        temp: 0.0,
        freq: cur_freq.max(0.0).round() as u64,
        act_freq: act_freq.max(0.0).round() as u64,
        min_freq: min_freq.max(0.0).round() as u64,
        max_freq: max_freq.max(0.0).round() as u64,
        activity_source: activity.source.to_string(),
        activity_detail: activity.detail.to_string(),
    })
}

fn first_existing_hwmon_temp(base: &str) -> f64 {
    let Ok(entries) = fs::read_dir(base) else {
        return 0.0;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let temp_path = path.join("temp1_input");
        if !temp_path.exists() {
            continue;
        }
        let temp = read_number(temp_path.to_string_lossy().as_ref()).unwrap_or(0.0);
        if temp > 0.0 {
            return temp / 1000.0;
        }
    }
    0.0
}

fn get_gpu(ctx: &Arc<AppContext>) -> Option<GpuInfo> {
    if let Ok(output) = Command::new("nvidia-smi")
        .arg("--query-gpu=utilization.gpu,utilization.memory,memory.used,memory.total,temperature.gpu,name")
        .arg("--format=csv,noheader,nounits")
        .output()
    {
        if output.status.success() {
            if let Ok(stdout) = String::from_utf8(output.stdout) {
                if let Some(line) = stdout.lines().next() {
                    let parts = line.split(',').map(|s| s.trim()).collect::<Vec<_>>();
                    if parts.len() >= 6 {
                        return Some(GpuInfo {
                            name: parts[5].to_string(),
                            utilization: parts[0].parse::<f64>().unwrap_or(0.0),
                            memory: parts[1].parse::<f64>().unwrap_or(0.0),
                            mem_used: parts[2].parse::<u64>().unwrap_or(0),
                            mem_total: parts[3].parse::<u64>().unwrap_or(1),
                            temp: parts[4].parse::<f64>().unwrap_or(0.0),
                            freq: 0,
                            act_freq: 0,
                            min_freq: 0,
                            max_freq: 0,
                            activity_source: "vendor-utilization".to_string(),
                            activity_detail: "Actual GPU utilization from nvidia-smi".to_string(),
                        });
                    }
                }
            }
        }
    }

    let amd_busy_path = "/sys/class/drm/card0/device/gpu_busy_percent";
    if Path::new(amd_busy_path).exists() {
        let amd_busy = read_i64(amd_busy_path).max(0) as f64;
        return Some(GpuInfo {
            name: "AMD GPU".to_string(),
            utilization: amd_busy,
            memory: 0.0,
            mem_used: 0,
            mem_total: 0,
            temp: first_existing_hwmon_temp("/sys/class/drm/card0/device/hwmon"),
            freq: 0,
            act_freq: 0,
            min_freq: 0,
            max_freq: 0,
            activity_source: "gpu_busy_percent".to_string(),
            activity_detail: "Actual GPU busy percent from amdgpu sysfs".to_string(),
        });
    }

    get_intel_gpu(ctx)
}

fn get_process_command(pid: i32, fallback: &str) -> String {
    let cmdline = read_text_file(&format!("/proc/{pid}/cmdline"));
    if let Some(cmd) = cmdline.split('\0').find(|part| !part.is_empty()) {
        return cmd.to_string();
    }

    if let Ok(path) = fs::read_link(format!("/proc/{pid}/exe")) {
        return path.to_string_lossy().to_string();
    }

    fallback.to_string()
}

fn get_top_bandwidth_processes(ctx: &Arc<AppContext>) -> Vec<TopBandwidthProcess> {
    let output = Command::new("ss")
        .arg("-tinpeoH")
        .arg("state")
        .arg("established")
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    let Ok(text) = String::from_utf8(output.stdout) else {
        return Vec::new();
    };

    let user_re =
        Regex::new(r#"users:\(\(\"([^\"]+)\",pid=(\d+),fd=\d+\)\)"#).expect("valid user regex");
    let stats_re =
        Regex::new(r"bytes_sent:(\d+).*bytes_received:(\d+)").expect("valid stats regex");

    struct PendingConn {
        name: String,
        command: String,
        pid: i32,
        key: String,
    }

    let mut grouped: HashMap<String, ProcessBandwidthCounter> = HashMap::new();
    let mut current: Option<PendingConn> = None;

    for raw_line in text.lines() {
        if raw_line.trim().is_empty() {
            continue;
        }

        if raw_line.starts_with(' ') || raw_line.starts_with('\t') {
            let Some(conn) = current.take() else {
                continue;
            };
            let Some(stats) = stats_re.captures(raw_line) else {
                continue;
            };

            let sent = stats[1].parse::<u64>().unwrap_or(0);
            let recv = stats[2].parse::<u64>().unwrap_or(0);

            let entry =
                grouped
                    .entry(conn.key.clone())
                    .or_insert_with(|| ProcessBandwidthCounter {
                        sent: 0,
                        recv: 0,
                        pid: conn.pid,
                        connections: 0,
                        name: conn.name.clone(),
                        command: conn.command.clone(),
                    });

            entry.sent = entry.sent.saturating_add(sent);
            entry.recv = entry.recv.saturating_add(recv);
            entry.connections = entry.connections.saturating_add(1);
            continue;
        }

        let Some(cap) = user_re.captures(raw_line) else {
            current = None;
            continue;
        };

        let name = cap[1].to_string();
        let pid = cap[2].parse::<i32>().unwrap_or(0);
        let command = get_process_command(pid, &name);
        current = Some(PendingConn {
            key: format!("{pid}|{command}"),
            name,
            command,
            pid,
        });
    }

    let now = Instant::now();
    let snapshot = grouped.clone();

    let mut telemetry = ctx.telemetry.lock().expect("telemetry mutex poisoned");
    let Some(prev) = telemetry.bandwidth_prev.replace(BandwidthSample {
        at: now,
        samples: snapshot,
    }) else {
        return Vec::new();
    };

    let dt = now.duration_since(prev.at).as_secs_f64().max(0.001);
    let mut result = Vec::new();

    for (key, sample) in grouped {
        let Some(previous) = prev.samples.get(&key) else {
            continue;
        };

        let d_sent = sample.sent.saturating_sub(previous.sent);
        let d_recv = sample.recv.saturating_sub(previous.recv);
        let tx_mbps = (d_sent as f64) * 8.0 / 1_000_000.0 / dt;
        let rx_mbps = (d_recv as f64) * 8.0 / 1_000_000.0 / dt;
        let total = rx_mbps + tx_mbps;
        if total <= 0.0 {
            continue;
        }

        result.push(TopBandwidthProcess {
            name: sample.name,
            command: sample.command,
            pid: sample.pid,
            connections: sample.connections,
            rx_mbps,
            tx_mbps,
            total_mbps: total,
        });
    }

    result.sort_by(|a, b| {
        b.total_mbps
            .partial_cmp(&a.total_mbps)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    result.truncate(20);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_percent_handles_nan_and_bounds() {
        assert_eq!(clamp_percent(f64::NAN), 0.0);
        assert_eq!(clamp_percent(-10.0), 0.0);
        assert_eq!(clamp_percent(42.5), 42.5);
        assert_eq!(clamp_percent(150.0), 100.0);
    }

    #[test]
    fn parse_cpu_total_from_stat_parses_cpu_line() {
        let sample = "cpu  100 5 20 200 10 0 0 0 0 0\ncpu0 10 1 2 20 1 0 0 0 0 0\n";
        let parsed = parse_cpu_total_from_stat(sample).expect("cpu line should parse");

        assert_eq!(parsed.user, 100);
        assert_eq!(parsed.nice, 5);
        assert_eq!(parsed.system, 20);
        assert_eq!(parsed.idle, 200);
        assert_eq!(parsed.iowait, 10);
    }

    #[test]
    fn parse_cpu_total_from_stat_returns_none_without_cpu_line() {
        let sample = "intr 1\nctxt 2\n";
        assert!(parse_cpu_total_from_stat(sample).is_none());
    }

    #[test]
    fn parse_core_times_from_stat_parses_per_core_entries() {
        let sample =
            "cpu  1 2 3 4 5 0 0 0 0 0\ncpu0 10 20 30 40 50 0 0 0 0 0\ncpu1 3 4 5 6 7 0 0 0 0 0\n";

        let parsed = parse_core_times_from_stat(sample);
        assert_eq!(parsed.len(), 2);
        assert!(parsed.contains_key("cpu0"));
        assert!(parsed.contains_key("cpu1"));

        let cpu0 = parsed.get("cpu0").expect("cpu0 must exist");
        assert_eq!(cpu0.user, 10);
        assert_eq!(cpu0.nice, 20);
        assert_eq!(cpu0.system, 30);
        assert_eq!(cpu0.idle, 40);
        assert_eq!(cpu0.iowait, 50);
        assert_eq!(cpu0.irq, 0);
        assert_eq!(cpu0.softirq, 0);
    }

    #[test]
    fn cpu_times_counts_irq_and_softirq_as_active_time() {
        let times = CpuTimes {
            user: 10,
            nice: 2,
            system: 3,
            idle: 80,
            iowait: 5,
            irq: 7,
            softirq: 11,
        };

        assert_eq!(times.active(), 33);
        assert_eq!(times.total(), 118);
    }

    #[test]
    fn parse_net_dev_totals_accumulates_rx_tx() {
        let sample = "Inter-|   Receive                                                |  Transmit\n face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n  lo: 100 0 0 0 0 0 0 0 100 0 0 0 0 0 0 0\neth0: 200 0 0 0 0 0 0 0 300 0 0 0 0 0 0 0\n";

        let (rx, tx) = parse_net_dev_totals(sample);
        assert_eq!(rx, 300);
        assert_eq!(tx, 400);
    }

    #[test]
    fn parse_net_dev_interfaces_returns_per_interface_map() {
        let sample = "Inter-|   Receive                                                |  Transmit\n face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n wlan0: 200 0 0 0 0 0 0 0 300 0 0 0 0 0 0 0\n  lo: 50 0 0 0 0 0 0 0 60 0 0 0 0 0 0 0\n";

        let parsed = parse_net_dev_interfaces(sample);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed.get("wlan0"), Some(&(200, 300)));
        assert_eq!(parsed.get("lo"), Some(&(50, 60)));
    }

    #[test]
    fn power_supply_watts_prefers_direct_power_now() {
        assert_eq!(
            power_supply_watts_from_values(12_345_000, 0, 0, 0, 0, 0, true),
            Some(12.345)
        );
    }

    #[test]
    fn power_supply_watts_can_derive_battery_voltage_and_current() {
        assert_eq!(
            power_supply_watts_from_values(0, 0, 5_000_000, 0, 3_250_000, 3_000_000, true),
            Some(16.25)
        );
    }

    #[test]
    fn power_supply_watts_does_not_derive_external_supply_limits() {
        assert_eq!(
            power_supply_watts_from_values(0, 0, 5_000_000, 0, 3_250_000, 3_000_000, false),
            None
        );
    }

    #[test]
    fn power_supply_watts_returns_none_without_voltage_or_current() {
        assert_eq!(
            power_supply_watts_from_values(0, 0, 0, 0, 3_000_000, 0, true),
            None
        );
        assert_eq!(
            power_supply_watts_from_values(0, 5_000_000, 0, 0, 0, 0, true),
            None
        );
    }

    #[test]
    fn first_existing_hwmon_temp_returns_zero_for_missing_base() {
        assert_eq!(first_existing_hwmon_temp("/definitely/missing/hwmon"), 0.0);
    }

    #[test]
    fn get_process_command_falls_back_for_invalid_pid() {
        assert_eq!(
            get_process_command(-1, "fallback-binary"),
            "fallback-binary"
        );
    }
}
