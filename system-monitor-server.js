#!/usr/bin/env node
const http = require('http');
const fs = require('fs');
const path = require('path');
const { execSync } = require('child_process');

const PORT = parseInt(process.env.PORT || '8765', 10);
const HTML = path.join(__dirname, 'system-monitor.html');

// ── helpers ──────────────────────────────────────────────────────────────────
function readFile(f) {
  try { return fs.readFileSync(f, 'utf8'); } catch { return ''; }
}
function readFileLines(f) { return readFile(f).split('\n'); }

function getCPU() {
  const prev = global._cpuPrev;
  const curr = parseCPUstat(readFileLines('/proc/stat'));
  if (!prev) { global._cpuPrev = curr; return 0; }
  const u = curr.u - prev.u, n = curr.n - prev.n, s = curr.s - prev.s,
        i = curr.i - prev.i, w = curr.w - prev.w;
  const t = u+n+s+i+w;
  global._cpuPrev = curr;
  return t > 0 ? ((u+n+s)*100/t) : 0;
}

function parseCPUstat(lines) {
  for (const l of lines) {
    if (!l.startsWith('cpu ')) continue;
    const p = l.split(/\s+/).slice(1,9).map(Number);
    return { u:p[0]||0, n:p[1]||0, s:p[2]||0, i:p[3]||0, w:p[4]||0, uu:p[5]||0, su:p[6]||0, x:p[7]||0 };
  }
  return { u:0,n:0,s:0,i:0,w:0,uu:0,su:0,x:0 };
}

function getCPUmodel() {
  return readFile('/proc/cpuinfo').split('\n').find(l=>l.startsWith('model name'))?.split(':')[1]?.trim() || 'Unknown';
}

function getCores() {
  const prev = global._coresPrev;
  const curr = parseCoreStat(readFileLines('/proc/stat'));
  if (!prev) { global._coresPrev = curr; return []; }
  const result = [];
  for (const k of Object.keys(curr)) {
    if (k === 'cpu') continue;
    const cu = curr[k], pu = prev[k];
    if (!pu) continue;
    const tu = cu.u-pu.u, tn = cu.n-pu.n, ts = cu.s-pu.s, ti = cu.i-pu.i, tw = cu.w-pu.w;
    const tt = tu+tn+ts+ti+tw;
    result.push({ label: k, pct: tt > 0 ? ((tu+tn+ts)*100/tt) : 0 });
  }
  global._coresPrev = curr;
  return result;
}

function parseCoreStat(lines) {
  const r = {};
  for (const l of lines) {
    if (!l.startsWith('cpu') || l.startsWith('cpu ')) continue;
    const p = l.split(/\s+/);
    const k = p[0];
    r[k] = { u: parseInt(p[1])|0, n: parseInt(p[2])|0, s: parseInt(p[3])|0, i: parseInt(p[4])|0, w: parseInt(p[5])|0 };
  }
  return r;
}

function getMem() {
  const mi = {};
  for (const l of readFileLines('/proc/meminfo')) {
    const m = l.match(/^(\w+):\s+(\d+)/);
    if (m) mi[m[1]] = parseInt(m[2]) * 1024;
  }
  const total = mi.MemTotal||0, avail = mi.MemAvailable||mi.MemFree||0, swapTotal = mi.SwapTotal||0, swapFree = mi.SwapFree||0;
  return {
    total, avail, used: total-avail,
    pct: total>0 ? ((total-avail)*100/total) : 0,
    swap: { total: swapTotal, free: swapFree, used: swapTotal-swapFree, pct: swapTotal>0 ? ((swapTotal-swapFree)*100/swapTotal) : 0 }
  };
}

function getGPU() {
  // Try NVIDIA first
  try {
    const out = execSync('nvidia-smi --query-gpu=utilization.gpu,utilization.memory,memory.used,memory.total,temperature.gpu,name --format=csv,noheader,nounits', {timeout:3000});
    const p = out.toString().trim().split(',').map(s=>s.trim());
    return {
      name: p[5]||'NVIDIA GPU',
      utilization: parseFloat(p[0])||0,
      memory: parseFloat(p[1])||0,
      memUsed: parseInt(p[2])||0,
      memTotal: parseInt(p[3])||1,
      temp: parseInt(p[4])||0
    };
  } catch {}
  // Try AMD
  try {
    const out = execSync('cat /sys/class/drm/card0/device/gpu_busy_percent 2>/dev/null', {timeout:2000});
    const util = parseInt(out.toString().trim())||0;
    const temp = parseInt(readFile('/sys/class/drm/card0/device/hwmon/hwmon1/temp1_input')||'0')/1000||0;
    return { name: 'AMD GPU', utilization: util, memory: 0, temp };
  } catch {}
  // Fallback: Intel via dedicated intel_gpu endpoint
  return getIntelGPU();
}

function getIntelGPU() {
  // Intel Iris Xe — read from sysfs (no root needed for frequency/RC6)
  const gpu = {
    name: 'Intel Iris Xe Graphics',
    utilization: 0,
    memory: 0,
    temp: 0,
    freq: 0,
    maxFreq: 1300
  };
  try {
    const curFreqPath = '/sys/class/drm/card1/gt/gt0/rps_cur_freq_mhz';
    const maxFreqPath = '/sys/class/drm/card1/gt/gt0/rps_max_freq_mhz';
    const minFreqPath = '/sys/class/drm/card1/gt/gt0/rps_min_freq_mhz';
    const actFreqPath = '/sys/class/drm/card1/gt/gt0/act_freq_mhz';
    const rc6Path = '/sys/class/drm/card1/gt/gt0/rc6_residency_ms';

    const curFreq = parseInt(readFile(curFreqPath)) || 0;
    const maxFreq = parseInt(readFile(maxFreqPath)) || 1300;
    const minFreq = parseInt(readFile(minFreqPath)) || 400;
    const actFreq = parseInt(readFile(actFreqPath)) || 0;
    const rc6Now = parseInt(readFile(rc6Path)) || 0;

    if (!global._gpuPrev) global._gpuPrev = { rc6: rc6Now, t: Date.now() };
    const prev = global._gpuPrev;
    const dt = (Date.now() - prev.t) / 1000;
    const rc6Delta = rc6Now - prev.rc6;
    prev.rc6 = rc6Now; prev.t = Date.now();

    // Frequency-based utilization: how high is the GPU clock relative to its range?
    const freqRange = maxFreq - minFreq;
    const freqPct = freqRange > 0 ? Math.min(100, ((curFreq - minFreq) * 100) / freqRange) : 0;
    // RC6 inverse: if RC6 delta ~= dt*1000, GPU was fully idle
    const idlePct = dt > 0 ? Math.min(100, (rc6Delta / (dt * 1000)) * 100) : 0;
    // Blend: 60% frequency signal, 40% inverse-RC6
    gpu.utilization = Math.round(freqPct * 0.6 + Math.max(0, 100 - idlePct) * 0.4);
    gpu.utilization = Math.max(0, Math.min(100, gpu.utilization));
    gpu.freq = curFreq;
    gpu.maxFreq = maxFreq;
    gpu.actFreq = actFreq;
  } catch(e) {
    // Not an Intel GPU or path doesn't exist
    return null;
  }
  return gpu;
}

function getNetwork() {
  const prev = global._netPrev;
  const now = Date.now();
  const lines = readFileLines('/proc/net/dev');
  let rx=0, tx=0;
  for (const l of lines.slice(2)) {
    const m = l.match(/^\s*\w+:\s+(\d+)\s+\d+\s+\d+\s+\d+\s+\d+\s+\d+\s+\d+\s+\d+\s+(\d+)/);
    if (m) { rx += parseInt(m[1]); tx += parseInt(m[2]); }
  }
  if (!prev) { global._netPrev = {rx,tx,t:now}; return {rx_bytes:0,tx_bytes:0}; }
  const dt = (now - prev.t) / 1000;
  const result = { rx_bytes: dt>0 ? (rx-prev.rx)/dt : 0, tx_bytes: dt>0 ? (tx-prev.tx)/dt : 0 };
  global._netPrev = {rx,tx,t:now};
  return result;
}

function getTopCPU() {
  try {
    const out = execSync('ps aux --sort=-%cpu --no-headers -ww | head -5', {timeout:3000});
    return out.toString().trim().split('\n').map(l=>{
      const p = l.trim().split(/\s+/);
      return { name: p[10]||p[0], cpu: parseFloat(p[2])||0 };
    }).filter(p=>p.cpu>0);
  } catch { return []; }
}

function getTopMem() {
  try {
    const out = execSync('ps aux --sort=-%mem --no-headers -ww | head -5', {timeout:3000});
    return out.toString().trim().split('\n').map(l=>{
      const p = l.trim().split(/\s+/);
      return { name: p[10]||p[0], mem: parseFloat(p[3])||0 };
    });
  } catch { return []; }
}

function getTopNet() {
  // Use /proc/net/dev per interface, combine with process I/O from /proc/PID/io
  try {
    const procs = execSync('ls /proc/ | grep "^[[:digit:]]"', {timeout:2000}).toString().trim().split('\n').slice(0,50);
    const results = [];
    for (const pid of procs) {
      try {
        const io = readFile(`/proc/${pid}/io`).trim().split('\n');
        let rChar=0, wChar=0;
        for (const l of io) {
          if (l.startsWith('read_bytes:')) rChar = parseInt(l.split(':')[1])||0;
          if (l.startsWith('write_bytes:')) wChar = parseInt(l.split(':')[1])||0;
        }
        const cmd = readFile(`/proc/${pid}/comm`).trim();
        results.push({ name: cmd, speed: rChar+wChar, pid });
      } catch {}
    }
    results.sort((a,b)=>b.speed-a.speed);
    return results.slice(0,5);
  } catch { return []; }
}

function getUptime() {
  return readFile('/proc/uptime').trim().split(' ')[0] || '0';
}

function getLoadavg() {
  return readFile('/proc/loadavg').trim();
}

function getBattery() {
  const base = '/sys/class/power_supply/BAT0';
  const adp = '/sys/class/power_supply/ADP1';
  const result = {
    status: 'Unknown',
    capacity: 0,
    percentage: 0,
    voltage: 0,
    current: 0,
    power: 0,
    charge_full: 0,
    charge_now: 0,
    cycle_count: 0,
    ac_online: false,
    time_remaining: null,
    charge_full_design: 0, // µAh — original factory capacity
    manufacturer: '',
    model_name: '',
  };
  try {
    result.status = readFile(base + '/status').trim();
    result.capacity = parseInt(readFile(base + '/capacity')) || 0;
    result.voltage = parseInt(readFile(base + '/voltage_now')) || 0; // µV
    result.current = Math.abs(parseInt(readFile(base + '/current_now')) || 0); // µA
    result.charge_full = parseInt(readFile(base + '/charge_full')) || 0; // µAh
    result.charge_now = parseInt(readFile(base + '/charge_now')) || 0; // µAh
    result.cycle_count = parseInt(readFile(base + '/cycle_count')) || 0;
    result.charge_full_design = parseInt(readFile(base + '/charge_full_design')) || 0;
    result.manufacturer = readFile(base + '/manufacturer').trim();
    result.model_name = readFile(base + '/model_name').trim();
    result.ac_online = parseInt(readFile(adp + '/online')) === 1;
    // Power = voltage * current (in µW, convert to W)
    result.power = (result.voltage * result.current) / 1e12; // W
    // Time remaining estimate
    if (result.current > 1000) { // Only calculate if discharging meaningfully
      if (result.status === 'Discharging') {
        result.time_remaining = Math.round(result.charge_now / result.current * 60); // minutes
      } else if (result.status === 'Charging') {
        result.time_remaining = Math.round((result.charge_full - result.charge_now) / result.current * 60);
      }
    }
  } catch(e) {}
  return result;
}

function getPower() {
  // Read RAPL energy counters to calculate package power
  const raplBase = '/sys/class/powercap/intel-rapl:0';
  const raplPkg = '/sys/class/powercap/intel-rapl:0:0';
  const now = Date.now();
  const result = { package: 0, core: 0, uncore: 0, dram: 0, total: 0 };
  
  try {
    // Read package energy (microjoules)
    const pkgEnergy = parseInt(readFile(raplPkg + '/energy_uj')) || 0;
    const coreEnergy = parseInt(readFile(raplPkg + '/intel-rapl:0:1/energy_uj')) || 0; // Might not exist
    
    if (!global._raplPrev) {
      global._raplPrev = { pkg: pkgEnergy, core: coreEnergy, t: now };
      return result;
    }
    
    const prev = global._raplPrev;
    const dt = (now - prev.t) / 1000; // seconds
    
    if (dt > 0.1) {
      const pkgDelta = pkgEnergy - prev.pkg;
      const coreDelta = coreEnergy - prev.core;
      
      // Handle rollover (energy_uj wraps around)
      const maxEnergy = 0xFFFFFFFF; // 32-bit counter typically
      const pkgDeltaFixed = pkgDelta < 0 ? (maxEnergy + pkgDelta) : pkgDelta;
      const coreDeltaFixed = coreDelta < 0 ? (maxEnergy + coreDelta) : coreDelta;
      
      result.package = pkgDeltaFixed / dt / 1e6; // Convert µJ to W
      result.core = coreDeltaFixed / dt / 1e6;
      result.total = result.package; // Package includes core + uncore + gpu
    }
    
    global._raplPrev = { pkg: pkgEnergy, core: coreEnergy, t: now };
  } catch(e) {}
  
  return result;
}

// ── API routes ────────────────────────────────────────────────────────────────
const routes = {
  '/api/system/stat': () => readFile('/proc/stat'),
  '/api/system/meminfo': () => readFile('/proc/meminfo'),
  '/api/system/memtotal': () => readFile('/proc/meminfo'),
  '/api/system/cpu_model': () => getCPUmodel(),
  '/api/system/gpu': () => JSON.stringify(getGPU()),
  '/api/system/intel_gpu': () => JSON.stringify(getIntelGPU()),
  '/api/system/network': () => JSON.stringify(getNetwork()),
  '/api/system/top_cpu': () => JSON.stringify(getTopCPU()),
  '/api/system/top_mem': () => JSON.stringify(getTopMem()),
  '/api/system/top_net': () => JSON.stringify(getTopNet()),
  '/api/system/uptime': () => getUptime(),
  '/api/system/loadavg': () => getLoadavg(),
  '/api/system/power': () => JSON.stringify(getPower()),
  '/api/system/battery': () => JSON.stringify(getBattery()),
  '/api/system/cores': () => {
    const cores = getCores();
    return cores.map(c=>`${c.pct.toFixed(1)}\t${c.label}`).join('\n');
  },
};

// ── server ───────────────────────────────────────────────────────────────────
const server = http.createServer((req, res) => {
  res.setHeader('Access-Control-Allow-Origin', '*');
  res.setHeader('Access-Control-Allow-Methods', 'GET');
  res.setHeader('Access-Control-Allow-Headers', 'Content-Type');

  // Strip query strings so cache-busting params don't break route matching
  const url = req.url.split('?')[0];

  if (req.method === 'OPTIONS') { res.writeHead(204); res.end(); return; }

  if (url === '/' || url === '/index.html') {
    fs.readFile(HTML, (err, data) => {
      if (err) { res.writeHead(500); res.end('Not found'); return; }
      res.writeHead(200, {'Content-Type': 'text/html; charset=utf-8'});
      res.end(data);
    });
    return;
  }

  // Auto-refresh endpoint — returns fresh data for non-browser clients
  if (url === '/refresh') {
    res.writeHead(200, {'Content-Type': 'application/json'});
    res.end(JSON.stringify({
      cpu: getCPU(),
      cpu_model: getCPUmodel(),
      cores: getCores(),
      mem: getMem(),
      gpu: getGPU(),
      network: getNetwork(),
      top_cpu: getTopCPU(),
      top_mem: getTopMem(),
      top_net: getTopNet(),
      uptime: getUptime(),
      loadavg: getLoadavg()
    }));
    return;
  }

  const handler = routes[url];
  if (handler) {
    try {
      const data = handler();
      const isJSON = url.startsWith('/api/system/gpu') || url.startsWith('/api/system/network') ||
                    url.startsWith('/api/system/top_') || url.startsWith('/api/system/cores');
      res.writeHead(200, {
        'Content-Type': isJSON ? 'application/json' : 'text/plain; charset=utf-8',
        'Cache-Control': 'no-store, no-cache, must-revalidate, proxy-revalidate',
        'Pragma': 'no-cache',
        'Expires': '0',
      });
      res.end(data);
    } catch (e) {
      res.writeHead(500); res.end(e.message);
    }
    return;
  }

  res.writeHead(404); res.end('Not found');
});

server.listen(PORT, '0.0.0.0', () => {
  console.log(`System Monitor: http://localhost:${PORT}`);
  console.log(`Serving: ${HTML}`);
});
