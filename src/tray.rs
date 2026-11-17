//! Значок в области уведомлений, как у оригинала: клик сворачивает и разворачивает окно,
//! правый клик — меню с управлением плеером.
//!
//! Только Windows: на Linux трей отложен (в GNOME без AppIndicator-расширения он
//! всё равно невидим), управление без окна даёт MPRIS (src/mpris.rs). Заглушки ниже
//! сохраняют прежнее API, чтобы `gui.rs` и `main.rs` не зависели от ОС.

use crate::sys::wide;
use std::cell::RefCell;
#[cfg(windows)]
use std::rc::Rc;
#[cfg(windows)]
use std::ptr::{null, null_mut};
#[cfg(windows)]
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
#[cfg(windows)]
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
#[cfg(windows)]
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK,
};
#[cfg(windows)]
use windows_sys::Win32::UI::Shell::{Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW};
#[cfg(windows)]
use windows_sys::Win32::UI::WindowsAndMessaging::*;

#[cfg(windows)]
const CLASS: &str = "yamusic_rs_tray";
/// Сообщения значка: клики и запрос на пробуждение от второго запуска.
#[cfg(windows)]
const WM_TRAY: u32 = WM_APP + 1;
/// Второй запуск просит уже работающий экземпляр показать окно.
#[cfg(windows)]
const WM_WAKE: u32 = WM_APP + 2;
/// Медиаклавиши клавиатуры: id горячей клавиши и код клавиши.
#[cfg(windows)]
const MEDIA_KEYS: [(i32, u16); 3] = [(1, VK_MEDIA_PLAY_PAUSE), (2, VK_MEDIA_NEXT_TRACK), (3, VK_MEDIA_PREV_TRACK)];

// Linux-сборка события и меню не читает (трея нет) — API сохранено для gui.rs.
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Toggle,
    Show,
    PlayPause,
    Prev,
    Next,
    Updates,
    Terms,
    Recommendations,
    Support,
    Quit,
}

/// Что показать в меню: окно свёрнуто или спрятано, играет ли трек, есть ли соседние.
// То же: поля читает только Windows-меню.
#[allow(dead_code)]
pub struct MenuState {
    pub minimized: bool,
    pub playing: bool,
    pub can_prev: bool,
    pub can_next: bool,
}

#[cfg(windows)]
struct Tray {
    hwnd: HWND,
    icon: HICON,
    taskbar_created: u32,
    on_event: Rc<dyn Fn(Event)>,
    state: Box<dyn Fn() -> MenuState>,
}

#[cfg(windows)]
thread_local! {
    static TRAY: RefCell<Option<Tray>> = const { RefCell::new(None) };
}

#[cfg(windows)]
fn load_icon() -> HICON {
    crate::sys::app_icon(unsafe { GetSystemMetrics(SM_CXSMICON) })
}

#[cfg(windows)]
fn notify_data(hwnd: HWND, icon: HICON) -> NOTIFYICONDATAW {
    let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;
    nid.hIcon = icon;
    let tip = wide(env!("APP_NAME"));
    for (d, s) in nid.szTip.iter_mut().zip(tip.iter().take(127)) {
        *d = *s;
    }
    nid
}

#[cfg(windows)]
fn add_icon(hwnd: HWND, icon: HICON) {
    let nid = notify_data(hwnd, icon);
    unsafe {
        Shell_NotifyIconW(NIM_ADD, &nid);
    }
}

/// Создаёт значок. События приходят в основной поток через очередь Slint,
/// а не из оконной процедуры: так их обработка не вкладывается в цикл winit.
#[cfg(windows)]
pub fn install(on_event: impl Fn(Event) + 'static, state: impl Fn() -> MenuState + 'static) {
    unsafe {
        let inst = GetModuleHandleW(null());
        let class = wide(CLASS);
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(wndproc);
        wc.hInstance = inst;
        wc.lpszClassName = class.as_ptr();
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(0, class.as_ptr(), class.as_ptr(), 0, 0, 0, 0, 0, null_mut(), null_mut(), inst, null());
        if hwnd.is_null() {
            return;
        }
        let icon = load_icon();
        let msg = wide("TaskbarCreated");
        let taskbar_created = RegisterWindowMessageW(msg.as_ptr());
        add_icon(hwnd, icon);
        // Глобально, чтобы клавиши работали и при свёрнутом окне. Если клавишу уже занял
        // другой плеер, RegisterHotKey просто вернёт ошибку — тогда она остаётся за ним.
        for (id, vk) in MEDIA_KEYS {
            RegisterHotKey(hwnd, id, 0, vk as u32);
        }
        TRAY.with(|t| {
            *t.borrow_mut() = Some(Tray { hwnd, icon, taskbar_created, on_event: Rc::new(on_event), state: Box::new(state) })
        });
    }
}

#[cfg(windows)]
pub fn remove() {
    let Some(t) = TRAY.with(|t| t.borrow_mut().take()) else { return };
    let nid = notify_data(t.hwnd, t.icon);
    unsafe {
        Shell_NotifyIconW(NIM_DELETE, &nid);
        for (id, _) in MEDIA_KEYS {
            UnregisterHotKey(t.hwnd, id);
        }
        DestroyWindow(t.hwnd);
    }
}

/// Для второго запуска: попросить работающий экземпляр показать окно.
#[cfg(windows)]
pub fn wake_existing() -> bool {
    let class = wide(CLASS);
    unsafe {
        let h = FindWindowW(class.as_ptr(), null());
        if h.is_null() {
            return false;
        }
        PostMessageW(h, WM_WAKE, 0, 0);
        // Иначе Windows не даст чужому окну выйти на передний план.
        windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(u32::MAX);
        true
    }
}

#[cfg(windows)]
fn emit(e: Event) {
    let _ = slint::invoke_from_event_loop(move || {
        let f = TRAY.with(|t| t.borrow().as_ref().map(|t| t.on_event.clone()));
        if let Some(f) = f {
            f(e);
        }
    });
}

#[cfg(windows)]
fn show_menu(hwnd: HWND) {
    let Some(s) = TRAY.with(|t| t.borrow().as_ref().map(|t| (t.state)())) else { return };
    let tr = crate::i18n::tr;
    unsafe {
        let menu = CreatePopupMenu();
        let about = CreatePopupMenu();
        let add = |m: HMENU, id: usize, text: &str, enabled: bool| {
            let w = wide(text);
            AppendMenuW(m, MF_STRING | if enabled { MF_ENABLED } else { MF_GRAYED }, id, w.as_ptr());
        };
        let sep = |m: HMENU| {
            AppendMenuW(m, MF_SEPARATOR, 0, null());
        };
        add(menu, 1, &tr(if s.minimized { "Развернуть" } else { "Свернуть" }), true);
        add(menu, 2, &tr("Проверить обновления"), true);
        sep(menu);
        add(menu, 3, &tr(if s.playing { "Пауза" } else { "Воспроизведение" }), true);
        add(menu, 4, &tr("Предыдущая песня"), s.can_prev);
        add(menu, 5, &tr("Следующая песня"), s.can_next);
        sep(menu);
        add(about, 6, &tr("Пользовательское соглашение"), true);
        add(about, 7, &tr("Правила рекомендаций"), true);
        add(about, 8, &tr("Чат с поддержкой"), true);
        let w = wide(&tr("О приложении"));
        AppendMenuW(menu, MF_POPUP, about as usize, w.as_ptr());
        sep(menu);
        add(menu, 9, &tr("Закрыть приложение"), true);

        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        // Без этого меню не закрывается кликом мимо.
        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, null());
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        let e = match cmd {
            1 => Event::Toggle,
            2 => Event::Updates,
            3 => Event::PlayPause,
            4 => Event::Prev,
            5 => Event::Next,
            6 => Event::Terms,
            7 => Event::Recommendations,
            8 => Event::Support,
            9 => Event::Quit,
            _ => return,
        };
        emit(e);
    }
}

#[cfg(windows)]
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            match lp as u32 {
                WM_LBUTTONUP => emit(Event::Toggle),
                WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
                _ => {}
            }
            0
        }
        WM_WAKE => {
            emit(Event::Show);
            0
        }
        WM_HOTKEY => {
            match wp {
                1 => emit(Event::PlayPause),
                2 => emit(Event::Next),
                3 => emit(Event::Prev),
                _ => {}
            }
            0
        }
        _ => {
            let recreate = TRAY.with(|t| t.borrow().as_ref().filter(|t| t.taskbar_created == msg && msg != 0).map(|t| t.icon));
            if let Some(icon) = recreate {
                add_icon(hwnd, icon);
                return 0;
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
    }
}

// ---------- Linux: заглушки, управление без окна даёт MPRIS ----------

#[cfg(not(target_os = "windows"))]
#[allow(dead_code)]
pub fn install(_on_event: impl Fn(Event) + 'static, _state: impl Fn() -> MenuState + 'static) {}

#[cfg(not(target_os = "windows"))]
#[allow(dead_code)]
pub fn remove() {}

#[cfg(not(target_os = "windows"))]
#[allow(dead_code)]
pub fn wake_existing() -> bool {
    false
}
