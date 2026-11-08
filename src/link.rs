//! 链路层：串口 / UDP。
//!
//! 模型：
//! - UI 线程同步打开链路（打开失败立即报错，不发状态）；
//! - 打开后把共享句柄 move 给只读线程，线程经 mpsc 回传字节 / 断线事件；
//! - 发送在 UI 线程直接用共享句柄（串口 write / UDP send_to）；
//! - 断开 = 置停止标志，接收线程退出。

use std::io::Write;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

/// UI 持有的发送句柄。
#[derive(Clone)]
pub enum LinkHandle {
    Serial(Arc<Mutex<Box<dyn serialport::SerialPort>>>),
    Udp(Arc<Mutex<UdpSocket>>),
}

/// 接收线程 → UI 事件。
pub enum LinkEvent {
    /// 收到的原始字节
    Data(Vec<u8>),
    /// 链路断开（人话原因）
    Lost(String),
}

/// 只读接收线程。
pub struct LinkWorker {
    pub events: mpsc::Receiver<LinkEvent>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl LinkWorker {
    fn spawn_reader<F>(&mut self, reader: F)
    where
        F: Fn(&AtomicBool, &mpsc::Sender<LinkEvent>) -> Option<String> + Send + 'static,
    {
        self.stop = Arc::new(AtomicBool::new(false));
        let stop = self.stop.clone();
        let (tx, events) = mpsc::channel::<LinkEvent>();
        self.events = events;
        self.handle = Some(std::thread::spawn(move || loop {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            if let Some(reason) = reader(&stop, &tx) {
                let _ = tx.send(LinkEvent::Lost(reason));
                return;
            }
        }));
    }

    /// 串口只读线程。
    pub fn spawn_serial(port: Arc<Mutex<Box<dyn serialport::SerialPort>>>) -> Self {
        let mut w = LinkWorker {
            events: mpsc::channel().1,
            stop: Arc::new(AtomicBool::new(false)),
            handle: None,
        };
        w.spawn_reader(move |stop, tx| {
            let mut buf = [0u8; 4096];
            loop {
                if stop.load(Ordering::SeqCst) {
                    return None;
                }
                let mut p = port.lock().unwrap();
                match p.read(&mut buf) {
                    Ok(0) => {}
                    Ok(n) => {
                        if tx.send(LinkEvent::Data(buf[..n].to_vec())).is_err() {
                            return None;
                        }
                    }
                    Err(e) => {
                        if e.kind() != std::io::ErrorKind::TimedOut
                            && e.kind() != std::io::ErrorKind::WouldBlock
                        {
                            return Some(format!("串口连接已断开 ({})", e));
                        }
                    }
                }
                drop(p);
                std::thread::sleep(Duration::from_millis(2));
            }
        });
        w
    }

    /// UDP 只读线程。
    pub fn spawn_udp(sock: Arc<Mutex<UdpSocket>>) -> Self {
        let mut w = LinkWorker {
            events: mpsc::channel().1,
            stop: Arc::new(AtomicBool::new(false)),
            handle: None,
        };
        w.spawn_reader(move |stop, tx| {
            let mut buf = [0u8; 65536];
            loop {
                if stop.load(Ordering::SeqCst) {
                    return None;
                }
                let s = sock.lock().unwrap();
                let _ = s.set_read_timeout(Some(Duration::from_millis(20)));
                match s.recv_from(&mut buf) {
                    Ok((n, _)) => {
                        if tx.send(LinkEvent::Data(buf[..n].to_vec())).is_err() {
                            return None;
                        }
                    }
                    Err(e) => {
                        if e.kind() != std::io::ErrorKind::TimedOut
                            && e.kind() != std::io::ErrorKind::WouldBlock
                        {
                            return Some(format!("UDP 连接已断开 ({})", e));
                        }
                    }
                }
                drop(s);
            }
        });
        w
    }

    /// 停止接收线程。
    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for LinkWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// 枚举本机串口（返回显示名列表）。
///
/// 显示格式对齐 Python 版 `f"{device} #{desc}"`（如 `COM7 #NPort Communication Port 1 (COM7)`）。
/// 描述来源：注册表 `SYSTEM\CurrentControlSet\Enum` 全量扫描 FriendlyName 以 `(COMx)` 结尾的
/// 设备实例 —— pyserial 的描述同源于此，覆盖 USB / MOXA NPort / VSPD 等所有驱动；
/// 注册表查不到的 USB 口回退 serialport 的 product 信息，都没有则只显示端口号。
#[cfg(windows)]
fn registry_friendly_names() -> std::collections::HashMap<String, String> {
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ};
    use winreg::RegKey;
    let mut map = std::collections::HashMap::new();
    let enum_root = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(r"SYSTEM\CurrentControlSet\Enum", KEY_READ);
    if let Ok(enum_root) = enum_root {
        for enumerator in enum_root.enum_keys().flatten() {
            let Ok(dev_root) = enum_root.open_subkey_with_flags(&enumerator, KEY_READ) else {
                continue;
            };
            for device in dev_root.enum_keys().flatten() {
                let Ok(inst_root) = dev_root.open_subkey_with_flags(&device, KEY_READ) else {
                    continue;
                };
                for instance in inst_root.enum_keys().flatten() {
                    let Ok(k) = inst_root.open_subkey_with_flags(&instance, KEY_READ) else {
                        continue;
                    };
                    let Ok(friendly) = k.get_value::<String, _>("FriendlyName") else {
                        continue;
                    };
                    // FriendlyName 形如 "NPort Communication Port 1 (COM7)"，
                    // 取末尾括号里的 COM 端口名做键
                    let Some(pos) = friendly.rfind("(COM") else {
                        continue;
                    };
                    let inner = &friendly[pos + 1..];
                    let Some(end) = inner.find(')') else {
                        continue;
                    };
                    let port = &inner[..end];
                    if !port.is_empty()
                        && port.chars().all(|c| c.is_ascii_alphanumeric())
                    {
                        map.insert(port.to_uppercase(), friendly);
                    }
                }
            }
        }
    }
    map
}

#[cfg(not(windows))]
fn registry_friendly_names() -> std::collections::HashMap<String, String> {
    std::collections::HashMap::new()
}

pub fn list_serial_ports() -> Vec<String> {
    let reg = registry_friendly_names();
    match serialport::available_ports() {
        Ok(ports) => ports
            .into_iter()
            .map(|p| {
                if let Some(friendly) = reg.get(&p.port_name.to_uppercase()) {
                    format!("{} #{}", p.port_name, friendly)
                } else {
                    match p.port_type {
                        serialport::SerialPortType::UsbPort(info) => format!(
                            "{} #{} (USB)",
                            p.port_name,
                            info.product.as_deref().unwrap_or("USB Device")
                        ),
                        _ => p.port_name,
                    }
                }
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// 打开串口（parity：None/Even/Odd 硬件校验位，修改后需重新连接生效）。
pub fn open_serial(port: &str, baud: u32, parity: serialport::Parity) -> Result<LinkHandle, String> {
    let p = serialport::new(port, baud)
        .timeout(Duration::from_millis(10))
        .parity(parity)
        .open()
        .map_err(|e| format!("无法打开串口 {}: {}", port, e))?;
    Ok(LinkHandle::Serial(Arc::new(Mutex::new(p))))
}

/// 绑定 UDP 监听（本机IP:本机端口）。
pub fn open_udp(bind_addr: &str, port: u16) -> Result<LinkHandle, String> {
    let s = UdpSocket::bind((bind_addr, port))
        .map_err(|e| format!("UDP 绑定 {}:{} 失败: {}", bind_addr, port, e))?;
    Ok(LinkHandle::Udp(Arc::new(Mutex::new(s))))
}

/// 发送字节（UI 线程调用）。
pub fn send_bytes(handle: &Option<LinkHandle>, data: &[u8], udp_target: Option<&str>) -> Result<(), String> {
    match handle {
        Some(LinkHandle::Serial(p)) => {
            let mut port = p.lock().unwrap();
            port.write_all(data).map_err(|e| format!("串口发送失败: {}", e))
        }
        Some(LinkHandle::Udp(s)) => {
            let sock = s.lock().unwrap();
            let target = udp_target.ok_or("未设置 UDP 目标地址")?;
            sock.send_to(data, target)
                .map(|_| ())
                .map_err(|e| format!("UDP 发送失败: {}", e))
        }
        None => Err("未连接".into()),
    }
}
