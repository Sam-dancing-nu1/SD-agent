//! 壳层诊断日志 + 控制台挂载（无黑窗后的唯一排障通道）。
//!
//! 双击启动时 exe 是 GUI 子系统（main.rs `windows_subsystem = "windows"`），
//! 没有控制台，stdout/stderr 无处可去。本模块提供两条诊断通道：
//! 1. 文件日志：始终追加写 <工作区>/.sd-agent/logs/desktop.log（失败静默降级，
//!    诊断通道自身不允许再制造故障）；
//! 2. 控制台开关：环境变量 SD_DESKTOP_CONSOLE=1 时挂回控制台
//!    （有父控制台就附着，没有就新开一个），开发期诊断用。

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

/// 控制台挂载结果（init 后固定；未 init 视为 false，只走文件）。
static CONSOLE: OnceLock<bool> = OnceLock::new();

/// 初始化：按开关挂控制台 + 注册 panic 落盘钩子（panic 也要留痕）。
pub fn init() {
    let attached = attach_console_if_requested();
    let _ = CONSOLE.set(attached);

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let msg = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(非字符串 panic 载荷)".to_string());
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "位置未知".to_string());
        log(format!("panic: {msg}（{location}）"));
        default_hook(info);
    }));
}

/// 写一行诊断日志：文件永远写；控制台挂上了才同步写 stderr。
pub fn log(msg: impl AsRef<str>) {
    let line = format!("[{}] {}\n", timestamp_utc(), msg.as_ref());
    if CONSOLE.get().copied().unwrap_or(false) {
        // 用 write_all + 忽略错误：禁止让诊断输出 panic。
        let _ = std::io::stderr().write_all(line.as_bytes());
    }
    let _ = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(log_dir())?;
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path())?;
        f.write_all(line.as_bytes())
    })();
}

fn log_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".sd-agent")
        .join("logs")
}

fn log_path() -> PathBuf {
    log_dir().join("desktop.log")
}

#[cfg(windows)]
fn attach_console_if_requested() -> bool {
    if std::env::var("SD_DESKTOP_CONSOLE").ok().as_deref() != Some("1") {
        return false;
    }
    win_console::attach_or_alloc()
}

#[cfg(not(windows))]
fn attach_console_if_requested() -> bool {
    false
}

/// Windows 控制台挂载：附着父控制台（开发期终端）或新开控制台（双击诊断）。
///
/// 只用 std 手写 FFI（壳层不加新依赖）；挂上后手动绑 CONIN$/CONOUT$ 到
/// 标准句柄——GUI 子系统进程的标准句柄初始为空，AttachConsole 不会自动补。
#[cfg(windows)]
mod win_console {
    use std::ptr;

    const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF; // (DWORD)-1
    const STD_INPUT_HANDLE: u32 = 0xFFFF_FFF6; // (DWORD)-10
    const STD_OUTPUT_HANDLE: u32 = 0xFFFF_FFF5; // (DWORD)-11
    const STD_ERROR_HANDLE: u32 = 0xFFFF_FFF4; // (DWORD)-12
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_SHARE_READ: u32 = 0x1;
    const FILE_SHARE_WRITE: u32 = 0x2;
    const OPEN_EXISTING: u32 = 3;

    unsafe extern "system" {
        fn AttachConsole(dw_process_id: u32) -> i32;
        fn AllocConsole() -> i32;
        fn SetStdHandle(n_std_handle: u32, handle: *mut core::ffi::c_void) -> i32;
        fn CreateFileW(
            lp_file_name: *const u16,
            dw_desired_access: u32,
            dw_share_mode: u32,
            lp_security_attributes: *mut core::ffi::c_void,
            dw_creation_disposition: u32,
            dw_flags_and_attributes: u32,
            h_template_file: *mut core::ffi::c_void,
        ) -> *mut core::ffi::c_void;
    }

    pub fn attach_or_alloc() -> bool {
        let attached = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) != 0 };
        if !attached && unsafe { AllocConsole() } == 0 {
            // 两个都拿不到控制台：静默降级，文件日志仍在。
            return false;
        }
        unsafe {
            let con_out = create_file(
                "CONOUT$",
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
            );
            if is_valid_handle(con_out) {
                SetStdHandle(STD_OUTPUT_HANDLE, con_out);
                SetStdHandle(STD_ERROR_HANDLE, con_out);
            }
            let con_in = create_file(
                "CONIN$",
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
            );
            if is_valid_handle(con_in) {
                SetStdHandle(STD_INPUT_HANDLE, con_in);
            }
        }
        true
    }

    fn create_file(name: &str, access: u32, share: u32) -> *mut core::ffi::c_void {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            CreateFileW(
                wide.as_ptr(),
                access,
                share,
                ptr::null_mut(),
                OPEN_EXISTING,
                0,
                ptr::null_mut(),
            )
        }
    }

    fn is_valid_handle(h: *mut core::ffi::c_void) -> bool {
        !h.is_null() && h != (-1isize as *mut core::ffi::c_void)
    }
}

/// UTC 时间戳（日志统一 UTC，避免本地时区换算引入依赖）。
fn timestamp_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}Z",
        tod / 3600,
        (tod % 3600) / 60,
        tod % 60
    )
}

/// 1970-01-01 起的天数 → (年, 月, 日)。标准 civil_from_days 算法。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::civil_from_days;

    /// 已知日期锚点：1970-01-01、2000-02-29（闰日）、2026-10-03。
    #[test]
    fn civil_from_days_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(11016), (2000, 2, 29));
        assert_eq!(civil_from_days(20729), (2026, 10, 3));
    }
}
