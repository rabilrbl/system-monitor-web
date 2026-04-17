use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CoreUsage {
    pub label: String,
    pub pct: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MemSwap {
    pub total: u64,
    pub free: u64,
    pub used: u64,
    pub pct: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MemInfo {
    pub total: u64,
    pub avail: u64,
    pub used: u64,
    pub pct: f64,
    pub swap: MemSwap,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GpuInfo {
    pub name: String,
    pub utilization: f64,
    pub memory: f64,
    #[serde(rename = "memUsed")]
    pub mem_used: u64,
    #[serde(rename = "memTotal")]
    pub mem_total: u64,
    pub temp: f64,
    pub freq: u64,
    #[serde(rename = "actFreq")]
    pub act_freq: u64,
    #[serde(rename = "minFreq")]
    pub min_freq: u64,
    #[serde(rename = "maxFreq")]
    pub max_freq: u64,
    pub activity_source: String,
    pub activity_detail: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct NetworkStats {
    pub rx_bytes: f64,
    pub tx_bytes: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TopCpuProcess {
    pub name: String,
    pub cpu: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TopMemProcess {
    pub name: String,
    pub mem: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TopNetInterface {
    pub name: String,
    pub rx_mbps: f64,
    pub tx_mbps: f64,
    pub speed: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TemperatureSensor {
    pub kind: String,
    pub id: String,
    pub label: String,
    pub source: String,
    pub value_c: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TopBandwidthProcess {
    pub name: String,
    pub command: String,
    pub pid: i32,
    pub connections: u32,
    pub rx_mbps: f64,
    pub tx_mbps: f64,
    pub total_mbps: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PowerStats {
    pub package: f64,
    pub core: f64,
    pub uncore: f64,
    pub dram: f64,
    pub total: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BatteryStats {
    pub status: String,
    pub capacity: i64,
    pub percentage: i64,
    pub voltage: i64,
    pub current: i64,
    pub power: f64,
    pub charge_full: i64,
    pub charge_now: i64,
    pub cycle_count: i64,
    pub ac_online: bool,
    pub time_remaining: Option<i64>,
    pub charge_full_design: i64,
    pub manufacturer: String,
    pub model_name: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RefreshPayload {
    pub cpu: f64,
    pub cpu_model: String,
    pub cores: Vec<CoreUsage>,
    pub mem: MemInfo,
    pub gpu: Option<GpuInfo>,
    pub network: NetworkStats,
    pub temperatures: Vec<TemperatureSensor>,
    pub top_cpu: Vec<TopCpuProcess>,
    pub top_mem: Vec<TopMemProcess>,
    pub top_net: Vec<TopNetInterface>,
    pub uptime: String,
    pub loadavg: String,
}
