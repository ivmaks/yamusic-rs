//! MPRIS 2 (D-Bus): виджет медиа в GNOME Shell (обложка, трек, ⏮▶⏭) и системные
//! мультимедиа-клавиши. Работает в отдельном потоке: `mpris_server::Player` не Send,
//! поэтому живёт в tokio current-thread рантайме и получает состояние плеера каналом.
//! В Windows-сборке — no-op-заглушки с тем же API.

use crate::api::Track;
use crate::player::Repeat;
#[cfg(not(target_os = "windows"))]
use crate::player::{Cmd, Player};
#[cfg(not(target_os = "windows"))]
use mpris_server::{LoopStatus, Metadata, PlaybackStatus, Player as MprisPlayer, Time, TrackId};
#[cfg(not(target_os = "windows"))]
use std::path::PathBuf;
#[cfg(not(target_os = "windows"))]
use std::sync::mpsc;

#[cfg(not(target_os = "windows"))]
/// Суффикс имени на шине D-Bus: org.mpris.MediaPlayer2.yamusic_rs.<suffix>.
const BUS_SUFFIX: &str = "yamusic_rs";
#[cfg(not(target_os = "windows"))]
/// Кэш обложки для mpris:artUrl — GNOME читает только локальные файлы.
const COVER_FILE: &str = "mpris-cover";
#[cfg(not(target_os = "windows"))]
/// Как часто сервер проверяет новые снапшоты.
const POLL: std::time::Duration = std::time::Duration::from_millis(50);

/// Снапшот плеера: шлём из тика GUI, сервер сам решает, что менять.
pub struct Snapshot {
    pub playing: bool,
    pub volume: f32,
    pub position: f32,
    pub repeat: Repeat,
    pub shuffle: bool,
    pub track: Option<Track>,
}

/// Отправитель снапшотов в MPRIS-поток. Существует всегда; без D-Bus — просто no-op.
#[derive(Clone, Default)]
pub struct Mpris {
    #[cfg(not(target_os = "windows"))]
    tx: Option<mpsc::Sender<Snapshot>>,
}

impl Mpris {
    /// Обновить состояние плеера. Вызывается из тика GUI раз в 250 мс.
    pub fn sync(&self, snap: Snapshot) {
        #[cfg(not(target_os = "windows"))]
        if let Some(tx) = &self.tx {
            let _ = tx.send(snap);
        }
        #[cfg(target_os = "windows")]
        {
            let _ = snap;
        }
    }
}

/// Поднять MPRIS-сервер в отдельном потоке. Не блокирует: нет D-Bus — не ошибка,
/// приложение работает без виджета медиа.
pub fn spawn(player: Player) -> Mpris {
    #[cfg(not(target_os = "windows"))]
    {
        let (tx, rx) = mpsc::channel::<Snapshot>();
        let ok = std::thread::Builder::new().name("mpris".into()).spawn(move || {
            if let Err(e) = run(player, rx) {
                crate::config::log(format!("mpris: {e}"));
            }
        });
        if ok.is_err() {
            return Mpris { tx: None };
        }
        Mpris { tx: Some(tx) }
    }
    #[cfg(target_os = "windows")]
    {
        let _ = player;
        Mpris {}
    }
}

#[cfg(not(target_os = "windows"))]
fn run(player: Player, rx: mpsc::Receiver<Snapshot>) -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime: {e}"))?;
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, server_task(player, rx))
}

async fn server_task(player: Player, rx: mpsc::Receiver<Snapshot>) -> Result<(), String> {
    // До входа и до готового плейлиста трека нет, и медиа-виджет в системе показывал бы
    // «неизвестного исполнителя». Поэтому имя на шине занимаем только на первом треке:
    // до этого момента приложения в списке медиаплееров нет вовсе.
    let first = loop {
        match rx.try_recv() {
            Ok(s) if s.track.is_some() => break s,
            Ok(_) => {}
            Err(mpsc::TryRecvError::Empty) => tokio::time::sleep(POLL).await,
            Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
        }
    };
    let p = MprisPlayer::builder(BUS_SUFFIX)
        .identity(env!("APP_NAME"))
        .desktop_entry("yamusic-rs")
        .can_quit(true)
        .can_raise(true)
        .can_play(true)
        .can_pause(true)
        .can_seek(true)
        .can_control(true)
        .can_go_previous(true)
        .can_go_next(true)
        .build()
        .await
        .map_err(|e| format!("d-bus: {e}"))?;

    // Кнопки GNOME Shell и медиа-клавиши → команды нашего плеера. Player: Clone + Send,
    // отправка в std-канал из async-контекста безопасна (неблокирующая). На каждый
    // обработчик — свой клон: замыкания забирают владение.
    let p_pause = player.clone();
    let p_pause2 = player.clone();
    let p_play = player.clone();
    let p_next = player.clone();
    let p_prev = player.clone();
    let p_seek = player.clone();
    let p_setpos = player.clone();
    let p_setpos_check = player.clone();
    let p_volume = player.clone();
    let p_loop = player.clone();
    let p_shuffle = player.clone();
    p.connect_play_pause(move |_| p_pause.send(Cmd::PlayPause));
    p.connect_play(move |_| p_play.send(Cmd::Resume));
    // Отдельный `Pause` нужен не для красоты: клавиши медиа на клавиатуре GNOME шлёт именно
    // `Pause`, а не `PlayPause` (видно по dbus-monitor: gsd-media-keys → Pause). Без этого
    // обработчика клавиша «плей/пауза» ничего не делала, хотя кнопки в уведомлении работали.
    p.connect_pause(move |_| p_pause2.send(Cmd::Pause));
    // «Стоп» в MPRIS — остановка воспроизведения; ближайшее по смыслу у нас — пауза.
    let p_stop = player.clone();
    p.connect_stop(move |_| p_stop.send(Cmd::Pause));
    p.connect_next(move |_| p_next.send(Cmd::Next));
    p.connect_previous(move |_| p_prev.send(Cmd::Prev));
    p.connect_seek(move |_, t| p_seek.send(Cmd::Seek(t.as_micros() as f32 / 1_000_000.0)));
    p.connect_set_position(move |_, id, t| {
        // «Устаревший» запрос: клиент не знает, что трек уже сменился, — игнорируем.
        if *id != track_path(p_setpos_check.state().track.as_ref()) {
            return;
        }
        p_setpos.send(Cmd::Seek(t.as_micros() as f32 / 1_000_000.0));
    });
    p.connect_set_volume(move |_, v| p_volume.send(Cmd::Volume(v as f32)));
    p.connect_set_loop_status(move |_, s| {
        let r = match s {
            LoopStatus::Track => Repeat::One,
            LoopStatus::Playlist => Repeat::All,
            LoopStatus::None => Repeat::Off,
        };
        p_loop.send(Cmd::Repeat(r));
    });
    p.connect_set_shuffle(move |_, on| p_shuffle.send(Cmd::Shuffle(on)));
    p.connect_raise(|_| {
        let _ = crate::gui::wake();
    });
    p.connect_quit(|_| {
        let _ = slint::quit_event_loop();
    });

    // Сервер живёт, пока жив процесс; цикл обновления свойств ждёт снапшоты.
    tokio::task::spawn_local(p.run());
    update_loop(p, rx, first).await;
    Ok(())
}

/// Обновление свойств: раз в POLL забираем снапшот и меняем только изменённое.
/// `first` — снапшот, на котором заняли имя на шине: применяем его до первого ожидания.
async fn update_loop(p: MprisPlayer, rx: mpsc::Receiver<Snapshot>, first: Snapshot) {
    let mut pending = Some(first);
    let mut last_track_id = String::new();
    let mut last_playing: Option<bool> = None;
    let mut last_volume = -1.0;
    let mut last_repeat = Repeat::default();
    let mut last_shuffle: Option<bool> = None;
    let mut last_pos = -1.0;
    let mut last_tick = std::time::Instant::now();
    loop {
        let s = match pending.take() {
            Some(s) => s,
            None => match rx.try_recv() {
                Ok(s) => s,
                Err(mpsc::TryRecvError::Empty) => {
                    tokio::time::sleep(POLL).await;
                    continue;
                }
                Err(mpsc::TryRecvError::Disconnected) => return,
            },
        };
        let track_id = s.track.as_ref().map(|t| t.full_id()).unwrap_or_default();
        if track_id != last_track_id {
            let was = !last_track_id.is_empty();
            last_track_id = track_id;
            // Сначала metadata без обложки, чтобы виджет не ждал сеть. Без трека
            // (выход из аккаунта) метаданные не сбрасываем: пустые поля GNOME показывает
            // как «неизвестного исполнителя», лучше оставить последний трек.
            if s.track.is_some() {
                let _ = p.set_metadata(metadata(&s.track, None)).await;
            }
            if was {
                // Смена трека: контроллеры перечитывают позицию после Seeked.
                last_pos = 0.0;
                last_tick = std::time::Instant::now();
                let _ = p.seeked(Time::from_millis(0)).await;
            }
            // Обложка качается в блокирующем пуле: update_loop на это время
            // припаркован, но D-Bus-задачи (p.run()) продолжают опрашиваться.
            let t = s.track.clone();
            let path = tokio::task::spawn_blocking(move || t.as_ref().and_then(cover_file)).await.ok().flatten();
            if let Some(path) = path {
                let _ = p.set_metadata(metadata(&s.track, Some(path))).await;
            }
        }
        if Some(s.playing) != last_playing {
            last_playing = Some(s.playing);
            let status = if s.playing { PlaybackStatus::Playing } else { PlaybackStatus::Paused };
            let _ = p.set_playback_status(status).await;
        }
        // Position PropertiesChanged не шлёт (по спецификации), читается по запросу.
        p.set_position(Time::from_millis((s.position * 1000.0) as i64));
        // Позиция в MPRIS: `Position` по спецификации не шлётся в PropertiesChanged.
        // Сигнал `Seeked` нужен на перемотке/переходе; чтобы кадр, пришедший с
        // опозданием (тик 250 мс), не выглядел перемоткой, сравниваем с ожидаемым
        // сдвигом: прошедшее время, пока играет.
        let now = std::time::Instant::now();
        let expected = last_pos + if s.playing { now.duration_since(last_tick).as_secs_f32() } else { 0.0 };
        if (s.position - expected).abs() > 1.0 {
            let _ = p.seeked(Time::from_millis((s.position * 1000.0) as i64)).await;
            last_pos = s.position;
        } else {
            last_pos = expected;
        }
        last_tick = now;
        if (s.volume - last_volume).abs() > 0.001 {
            last_volume = s.volume;
            let _ = p.set_volume(f64::from(s.volume)).await;
        }
        if s.repeat != last_repeat {
            last_repeat = s.repeat;
            let ls = match s.repeat {
                Repeat::One => LoopStatus::Track,
                Repeat::All => LoopStatus::Playlist,
                Repeat::Off => LoopStatus::None,
            };
            let _ = p.set_loop_status(ls).await;
        }
        if Some(s.shuffle) != last_shuffle {
            last_shuffle = Some(s.shuffle);
            let _ = p.set_shuffle(s.shuffle).await;
        }
    }
}

/// Путь трека для MPRIS (`mpris:trackid`): по нему контроллеры (в том числе расширения
/// GNOME, рисующие полосу прогресса) вызывают `SetPosition`. Спецификация требует это поле
/// в каждый момент, поэтому без трека отдаём `NoTrack`.
fn track_path(t: Option<&Track>) -> TrackId {
    let Some(t) = t else { return TrackId::NO_TRACK };
    let id: String = t.full_id().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    TrackId::try_from(format!("/org/yamusic/Track/{id}")).unwrap_or(TrackId::NO_TRACK)
}

fn metadata(track: &Option<Track>, art: Option<PathBuf>) -> Metadata {
    let Some(t) = track else { return Metadata::new() };
    let mut md = Metadata::builder()
        .trackid(track_path(Some(t)))
        .title(format!("{}{}", t.title, if t.version.is_empty() { String::new() } else { format!(" ({})", t.version) }))
        .artist(t.artists.split(", ").map(String::from).collect::<Vec<_>>())
        .album(t.album_title.clone())
        .length(Time::from_millis(t.duration_ms as i64));
    if let Some(path) = art {
        md = md.art_url(format!("file://{}", path.display()));
    }
    md.build()
}

/// Скачать обложку трека в файл кэша; обложка меняется только со сменой трека.
fn cover_file(t: &Track) -> Option<PathBuf> {
    let url = t.cover_url(400)?;
    let bytes = reqwest::blocking::Client::new()
        .get(url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .ok()?
        .error_for_status()
        .ok()?
        .bytes()
        .ok()?;
    let dir = crate::config::dir();
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(COVER_FILE);
    std::fs::write(&path, &bytes).ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: &str, album: Option<&str>) -> Track {
        Track {
            id: id.into(),
            album_id: album.map(String::from),
            title: String::new(),
            version: String::new(),
            artists: String::new(),
            artist_list: Vec::new(),
            duration_ms: 0,
            cover: None,
            explicit: false,
            lyrics: false,
            color: None,
            album_title: String::new(),
            year: None,
            position: None,
        }
    }

    #[test]
    fn trackid_is_a_valid_object_path() {
        let p = track_path(Some(&track("12345", Some("678"))));
        assert!(p.as_str().starts_with("/org/yamusic/Track/"));
        assert!(p.as_str().chars().all(|c| c.is_ascii_alphanumeric() || c == '/' || c == '_'));
        // Разные треки — разные пути, иначе SetPosition попадёт не в тот трек.
        assert_ne!(track_path(Some(&track("1", None))), track_path(Some(&track("2", None))));
        assert_eq!(track_path(None), TrackId::NO_TRACK);
    }
}
