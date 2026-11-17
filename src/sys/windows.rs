//! Windows: тонкие обёртки над Win32 — окно, фокус, мониторы, диалоги, автозапуск, один экземпляр.

use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentProcess, GetCurrentThreadId, SetProcessWorkingSetSize};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows_sys::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

fn h(x: isize) -> HWND {
    x as HWND
}

/// HWND окна Slint; 0, пока winit его не создал.
fn handle(w: &slint::Window) -> isize {
    use slint::winit_030::{winit::raw_window_handle::{HasWindowHandle, RawWindowHandle}, WinitWindowAccessor};
    w.with_winit_window(|w| match w.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
        _ => 0,
    })
    .unwrap_or(0)
}

/// Окно уже создано: winit делает это не сразу после `show()`.
pub fn created(w: &slint::Window) -> bool {
    handle(w) != 0
}

/// Один раз после создания окна: скруглённые углы, значок, HWND для второго запуска.
pub fn prepare(w: &slint::Window) {
    let hwnd = handle(w);
    round_corners(hwnd);
    let _ = std::fs::write(crate::config::dir().join("hwnd"), hwnd.to_string());
    set_icon(hwnd);
}

pub fn raise(w: &slint::Window) {
    let hwnd = handle(w);
    show(hwnd);
    force_foreground(hwnd);
}

pub fn is_shown(w: &slint::Window) -> bool {
    let hwnd = handle(w);
    hwnd != 0 && is_visible(hwnd)
}

pub fn is_minimized(w: &slint::Window) -> bool {
    is_iconic(handle(w))
}

/// Масштаб монитора, на котором окно.
pub fn scale(w: &slint::Window) -> f32 {
    dpi_scale(handle(w))
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn is_visible(hwnd: isize) -> bool {
    unsafe { IsWindowVisible(h(hwnd)) != 0 }
}

fn is_iconic(hwnd: isize) -> bool {
    unsafe { IsIconic(h(hwnd)) != 0 }
}

fn show(hwnd: isize) {
    unsafe {
        if IsIconic(h(hwnd)) != 0 {
            ShowWindow(h(hwnd), SW_RESTORE);
        } else {
            ShowWindow(h(hwnd), SW_SHOW);
        }
    }
}

pub fn hide(w: &slint::Window) {
    unsafe {
        ShowWindow(h(handle(w)), SW_HIDE);
    }
}

/// SetForegroundWindow с обходом ограничений Windows на кражу фокуса.
fn force_foreground(hwnd: isize) {
    unsafe {
        let target = h(hwnd);
        if SetForegroundWindow(target) != 0 && GetForegroundWindow() == target {
            return;
        }
        let fg = GetForegroundWindow();
        let fg_thread = GetWindowThreadProcessId(fg, null_mut());
        let me = GetCurrentThreadId();
        if fg_thread != 0 && fg_thread != me {
            AttachThreadInput(me, fg_thread, 1);
            BringWindowToTop(target);
            SetForegroundWindow(target);
            AttachThreadInput(me, fg_thread, 0);
        } else {
            BringWindowToTop(target);
        }
    }
}

fn dpi_scale(hwnd: isize) -> f32 {
    let dpi = unsafe { GetDpiForWindow(h(hwnd)) };
    if dpi == 0 {
        1.0
    } else {
        dpi as f32 / 96.0
    }
}

/// Значок из ресурсов нашего exe (кладёт build.rs), иначе стандартный.
pub fn app_icon(size: i32) -> HICON {
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    unsafe {
        let icon = LoadImageW(GetModuleHandleW(null()), 1 as _, IMAGE_ICON, size, size, LR_DEFAULTCOLOR);
        if !icon.is_null() {
            return icon as HICON;
        }
        LoadIconW(null_mut(), IDI_APPLICATION)
    }
}

/// Значок окна в заголовке, на панели задач и в Alt+Tab.
fn set_icon(hwnd: isize) {
    unsafe {
        let big = app_icon(GetSystemMetrics(SM_CXICON));
        let small = app_icon(GetSystemMetrics(SM_CXSMICON));
        SendMessageW(h(hwnd), WM_SETICON, ICON_BIG as usize, big as isize);
        SendMessageW(h(hwnd), WM_SETICON, ICON_SMALL as usize, small as isize);
    }
}

/// Скруглённые углы окна на Windows 11 (на 10 просто ничего не делает).
fn round_corners(hwnd: isize) {
    let pref = DWMWCP_ROUND;
    unsafe {
        DwmSetWindowAttribute(
            h(hwnd),
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            &pref as *const _ as *const c_void,
            std::mem::size_of_val(&pref) as u32,
        );
    }
}

/// Системный диалог выбора картинки (JPEG, PNG). `None` — пользователь отказался.
pub fn pick_image(owner: &slint::Window, title: &str) -> Option<String> {
    use windows_sys::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_PATHMUSTEXIST, OPENFILENAMEW};
    let filter: Vec<u16> = "Изображения (*.jpg, *.png)\0*.jpg;*.jpeg;*.png\0\0".encode_utf16().collect();
    let title = wide(title);
    let mut buf = vec![0u16; 1024];
    unsafe {
        let mut ofn: OPENFILENAMEW = std::mem::zeroed();
        ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
        ofn.hwndOwner = h(handle(owner));
        ofn.lpstrFilter = filter.as_ptr();
        ofn.lpstrFile = buf.as_mut_ptr();
        ofn.nMaxFile = buf.len() as u32;
        ofn.lpstrTitle = title.as_ptr();
        ofn.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST;
        if GetOpenFileNameW(&mut ofn) == 0 {
            return None;
        }
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..n]))
}

/// Системный диалог выбора папки, открывается на `start`. `None` — пользователь отказался.
pub fn pick_folder(owner: &slint::Window, title: &str, start: &str) -> Option<String> {
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{
        SHBrowseForFolderW, SHGetPathFromIDListW, BFFM_INITIALIZED, BFFM_SETSELECTIONW, BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS, BROWSEINFOW,
    };
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    fn h(x: isize) -> HWND {
        x as HWND
    }
    let title = wide(title);
    let start = wide(start);
    let mut buf = vec![0u16; 1024];
    unsafe {
        let mut bi: BROWSEINFOW = std::mem::zeroed();
        bi.hwndOwner = h(handle(owner));
        bi.lpszTitle = title.as_ptr();
        bi.ulFlags = BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE;
        bi.lpfn = Some(on_init);
        bi.lParam = start.as_ptr() as isize;
        let pidl = SHBrowseForFolderW(&bi);
        if pidl.is_null() {
            return None;
        }
        let ok = SHGetPathFromIDListW(pidl, buf.as_mut_ptr());
        CoTaskMemFree(pidl as *const c_void);
        if ok == 0 {
            return None;
        }
    }

/// Рабочая область монитора с окном (без панели задач), в физических пикселях: лево, верх, право, низ.
pub fn work_area(w: &slint::Window) -> (i32, i32, i32, i32) {
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    unsafe {
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(MonitorFromWindow(h(handle(w)), MONITOR_DEFAULTTONEAREST), &mut mi);
        let r = mi.rcWork;
        (r.left, r.top, r.right, r.bottom)
    }
}

/// Linux: то же поведение через XDG. Оконные функции — no-op: ими управляет
/// композитор, а `gui.rs` прячет/показывает окно через API Slint. Но-опы оставлены
/// ради паритета API с Win32-веткой.
#[cfg(not(target_os = "windows"))]
#[allow(dead_code)]
pub use linux::*;

#[cfg(not(target_os = "windows"))]
#[allow(dead_code)]
mod linux {
    const APP_ID: &str = "yamusic-rs";

    // ---------- окно (композитор решает сам) ----------

    pub fn is_visible(_hwnd: isize) -> bool {
        false
    }

    pub fn is_iconic(_hwnd: isize) -> bool {
        false
    }

    pub fn show(_hwnd: isize) {}

    pub fn hide(_hwnd: isize) {}

    pub fn force_foreground(_hwnd: isize) {}

    pub fn dpi_scale(_hwnd: isize) -> f32 {
        1.0
    }

    pub fn app_icon(_size: i32) {}

    pub fn set_icon(_hwnd: isize) {}

    pub fn round_corners(_hwnd: isize) {}

    pub fn is_window(_hwnd: isize) -> bool {
        false
    }

    // ---------- системные мелочи ----------

    /// Диалог выбора картинки через xdg-desktop-portal (rfd). `owner` не нужен.
    pub fn pick_image(_owner: isize, title: &str) -> Option<String> {
        rfd::FileDialog::new()
            .set_title(title)
            .add_filter(crate::i18n::tr("Изображения"), &["jpg", "jpeg", "png"])
            .pick_file()
            .map(|p| p.display().to_string())
    }

    /// Системный диалог выбора папки (xdg-desktop-portal через rfd).
    pub fn pick_folder(_owner: isize, title: &str, start: &str) -> Option<String> {
        let mut d = rfd::FileDialog::new().set_title(title);
        if !start.is_empty() {
            d = d.set_directory(start);
        }
        d.pick_folder().map(|p| p.display().to_string())
    }

    /// Открыть ссылку в браузере по умолчанию. Ребёнка не ждём в вызывающем потоке
    /// (это поток интерфейса): на некоторых системах `xdg-open` живёт до закрытия
    /// браузера. Ждём в фоне — так не остаётся зомби и не замирает окно.
    pub fn open_url(url: &str) {
        // Без `sh -c`: аргумент передаётся как есть, кавычки и экранирование не нужны.
        match std::process::Command::new("xdg-open")
            .arg(url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(mut child) => {
                let url = url.to_string();
                let _ = std::thread::Builder::new().name("open-url".into()).spawn(move || match child.wait() {
                    Ok(s) if s.success() => {}
                    Ok(s) => crate::config::log(format!("xdg-open {url}: код {}", s.code().unwrap_or(-1))),
                    Err(e) => crate::config::log(format!("xdg-open {url}: {e}")),
                });
            }
            Err(e) => crate::config::log(format!("xdg-open: {e}")),
        }
    }

    /// На Linux нечего отдавать ОС: память вернёт аллокатор/композитор.
    pub fn trim_memory() {}

    pub fn release_memory() {}

    /// Светлая ли тема: спрашиваем GNOME (`color-scheme`). Всё остальное — тёмная
    /// (у приложения тёмный дизайн), как и «system» без окружения GNOME.
    pub fn system_light_theme() -> bool {
        match std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "color-scheme"])
            .output()
        {
            Ok(o) if o.status.success() => {
                let s = String::from_utf8_lossy(&o.stdout);
                s.contains("prefer-light")
            }
            _ => false,
        }
    }

    // ---------- автозапуск: ~/.config/autostart/yamusic-rs.desktop ----------

    fn autostart_file() -> Option<std::path::PathBuf> {
        // ~/.config/autostart — родитель каталога данных приложения (config::dir).
        crate::config::dir().parent().map(|p| p.join("autostart").join(format!("{APP_ID}.desktop")))
    }

    fn autostart_command() -> Option<String> {
        Some(format!("{} --hidden", std::env::current_exe().ok()?.display()))
    }

    fn write_desktop(exec: &str) -> std::io::Result<()> {
        let Some(path) = autostart_file() else { return Err(std::io::ErrorKind::NotFound.into()) };
        std::fs::create_dir_all(path.parent().unwrap())?;
        let body = format!(
            "[Desktop Entry]\nType=Application\nName={}\nExec={}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
            env!("APP_NAME"),
            exec
        );
        std::fs::write(&path, body)
    }

    /// Записан ли автозапуск вместе с системой.
    pub fn autostart_enabled() -> bool {
        autostart_file().map(|p| p.exists()).unwrap_or(false)
    }

    /// Включить или выключить автозапуск (файл в ~/.config/autostart).
    pub fn set_autostart(on: bool) -> bool {
        if !on {
            return autostart_file().map(|p| std::fs::remove_file(p).is_ok()).unwrap_or(false);
        }
        let Some(cmd) = autostart_command() else { return false };
        write_desktop(&cmd).is_ok()
    }

    /// Бинарь переехал — переписать Exec, если автозапуск включён.
    pub fn refresh_autostart() {
        if autostart_enabled() {
            set_autostart(true);
        }
    }
}

/// Exe перенесли в другую папку — поправить путь в автозапуске, если он включён.
pub fn refresh_autostart() {
    if autostart_enabled() {
        set_autostart(true);
    }
}

fn is_window(hwnd: isize) -> bool {
    unsafe { IsWindow(h(hwnd)) != 0 }
}

/// Занять мьютекс единственного экземпляра. Если он уже чей-то — отпустить свою ссылку,
/// чтобы мьютекс исчез вместе с тем процессом.
pub fn claim_instance() -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    let name = wide(r"Local\yamusic_rs_single_instance");
    unsafe {
        let m = CreateMutexW(null(), 0, name.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            CloseHandle(m);
            return false;
        }
        true
    }
}

/// Второй запуск: показать окно работающего экземпляра.
pub fn wake_existing() {
    if crate::tray::wake_existing() {
        return;
    }
    let hwnd: isize = std::fs::read_to_string(crate::config::dir().join("hwnd")).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    if hwnd != 0 && is_window(hwnd) {
        show(hwnd);
        force_foreground(hwnd);
    }
}

/// Перед выходом: убрать значок из трея и HWND для второго запуска.
pub fn cleanup() {
    crate::tray::remove();
    let _ = std::fs::remove_file(crate::config::dir().join("hwnd"));
}

pub fn config_base() -> PathBuf {
    PathBuf::from(std::env::var("APPDATA").unwrap_or_else(|_| ".".into()))
}

pub fn music_dir() -> PathBuf {
    PathBuf::from(std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into())).join("Music")
}

pub fn hostname() -> Option<String> {
    std::env::var("COMPUTERNAME").ok()
}

/// Скопировать текст в буфер обмена.
pub fn copy_text(text: &str) {
    use std::io::Write as _;
    use std::os::windows::process::CommandExt;
    if let Ok(mut child) = std::process::Command::new("clip")
        .creation_flags(0x0800_0000)
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
}
