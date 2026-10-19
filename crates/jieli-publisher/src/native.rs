use std::io::Cursor;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pipewire as pw;
use pw::spa;
use pw::spa::param::format::{FormatProperties, MediaSubtype, MediaType};
use pw::spa::param::video::VideoFormat;
use pw::spa::pod::{ChoiceValue, Object, Pod, Property, PropertyFlags, Value};
use pw::spa::utils::{Choice, ChoiceEnum, ChoiceFlags, Fraction, Id, Rectangle, SpaTypes};

use jieli_core::frame::{Frame, HEIGHT, UV_SIZE, WIDTH, Y_SIZE};
use jieli_core::placeholder::placeholder;

use crate::feed::PublisherFeed;

const FPS: u32 = 30;
const FRAME_INTERVAL_NS: u64 = 33_333_333;

#[derive(Default)]
struct Stats {
    streaming: AtomicBool,
    frames: AtomicU64,
    eagain: AtomicU64,
    ebusy: AtomicU64,
    epipe: AtomicU64,
    other_errno: AtomicU64,
    last_errno: AtomicI32,
    max_datas: AtomicU32,
    short_buffers: AtomicU64,
    process_calls: AtomicU64,
    trigger_errors: AtomicU64,
    timer_ticks: AtomicU64,
}

pub struct NativeOptions<'a> {
    pub node_name: &'a str,
    pub description: &'a str,
    pub feed: Arc<Mutex<PublisherFeed>>,
    pub quit: &'static AtomicBool,
    pub failed: &'static AtomicBool,
    pub debug_requested: &'static AtomicBool,
    pub debug_until_ms: &'static AtomicU64,
}

struct UserData {
    format: spa::param::video::VideoInfoRaw,
    feed: Arc<Mutex<PublisherFeed>>,
    stats: Arc<Stats>,
    published: Arc<AtomicU64>,
    quit: &'static AtomicBool,
    failed: &'static AtomicBool,
    logged_layout: bool,
}

fn prop(key: u32, value: Value) -> Property {
    Property {
        key,
        flags: PropertyFlags::empty(),
        value,
    }
}

fn serialize(object: Object) -> Vec<u8> {
    spa::pod::serialize::PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(object))
        .expect("serialize pod")
        .0
        .into_inner()
}

fn format_param() -> Vec<u8> {
    let object = spa::pod::object!(
        SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        spa::pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        spa::pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        spa::pod::property!(FormatProperties::VideoFormat, Id, VideoFormat::I420),
        spa::pod::property!(
            FormatProperties::VideoSize,
            Rectangle,
            Rectangle {
                width: WIDTH as u32,
                height: HEIGHT as u32
            }
        ),
        spa::pod::property!(
            FormatProperties::VideoFramerate,
            Fraction,
            Fraction { num: FPS, denom: 1 }
        ),
    );
    serialize(object)
}

fn buffers_param(width: u32, height: u32) -> Vec<u8> {
    let y_stride = width;
    let uv_stride = width.div_ceil(2);
    let y_size = y_stride * height;
    let uv_size = uv_stride * height.div_ceil(2);
    let total = y_size + 2 * uv_size;

    let object = spa::pod::object!(
        SpaTypes::ObjectParamBuffers,
        spa::param::ParamType::Buffers,
        prop(
            libspa_sys::SPA_PARAM_BUFFERS_buffers,
            Value::Choice(ChoiceValue::Int(Choice(
                ChoiceFlags::empty(),
                ChoiceEnum::Range {
                    default: 8,
                    min: 2,
                    max: 64
                }
            )))
        ),
        prop(libspa_sys::SPA_PARAM_BUFFERS_blocks, Value::Int(3)),
        prop(libspa_sys::SPA_PARAM_BUFFERS_size, Value::Int(total as i32)),
        prop(
            libspa_sys::SPA_PARAM_BUFFERS_stride,
            Value::Int(y_stride as i32)
        ),
        prop(
            libspa_sys::SPA_PARAM_BUFFERS_dataType,
            Value::Choice(ChoiceValue::Int(Choice(
                ChoiceFlags::empty(),
                ChoiceEnum::Flags {
                    default: 1i32 << (libspa_sys::SPA_DATA_MemFd),
                    flags: vec![
                        1i32 << (libspa_sys::SPA_DATA_MemFd),
                        1i32 << (libspa_sys::SPA_DATA_MemPtr),
                    ],
                }
            )))
        ),
    );
    serialize(object)
}

fn meta_param() -> Vec<u8> {
    let object = spa::pod::object!(
        SpaTypes::ObjectParamMeta,
        spa::param::ParamType::Meta,
        prop(
            libspa_sys::SPA_PARAM_META_type,
            Value::Id(Id(libspa_sys::SPA_META_Header))
        ),
        prop(
            libspa_sys::SPA_PARAM_META_size,
            Value::Int(std::mem::size_of::<libspa_sys::spa_meta_header>() as i32)
        ),
    );
    serialize(object)
}

fn fill_frame(raw: *mut pw::sys::pw_buffer, frame: &Frame, stats: &Stats) {
    let sizes = [Y_SIZE, UV_SIZE, UV_SIZE];
    let strides = [WIDTH as i32, (WIDTH / 2) as i32, (WIDTH / 2) as i32];

    unsafe {
        let buffer = (*raw).buffer;
        if buffer.is_null() {
            return;
        }
        let count = (*buffer).n_datas as usize;
        stats.max_datas.fetch_max(count as u32, Ordering::Relaxed);
        let source = frame.as_bytes();
        let mut source_offset = 0usize;

        for index in 0..count.min(3) {
            let data = &mut *(*buffer).datas.add(index);
            let wanted = sizes[index];
            let maximum = data.maxsize as usize;
            if maximum < wanted {
                stats.short_buffers.fetch_add(1, Ordering::Relaxed);
            }
            let length = wanted.min(maximum);
            if !data.data.is_null() && length > 0 && source_offset + length <= source.len() {
                std::ptr::copy_nonoverlapping(
                    source.as_ptr().add(source_offset),
                    data.data as *mut u8,
                    length,
                );
            }
            source_offset += wanted;

            if !data.chunk.is_null() {
                let chunk = &mut *data.chunk;
                chunk.offset = 0;
                chunk.size = length as u32;
                chunk.stride = strides[index];
                chunk.flags = 0;
            }
        }
    }
}

fn monotonic_millis() -> u64 {
    let mut timestamp: libc::timespec = unsafe { std::mem::zeroed() };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut timestamp) };
    timestamp.tv_sec as u64 * 1000 + timestamp.tv_nsec as u64 / 1_000_000
}

/// Publish frames until EOF, quit signal or stream error.
pub fn run(options: NativeOptions<'_>) -> Result<(), String> {
    pw::init();

    let mainloop =
        pw::main_loop::MainLoopRc::new(None).map_err(|error| format!("main loop: {error:?}"))?;
    let context = pw::context::ContextRc::new(&mainloop, None)
        .map_err(|error| format!("context: {error:?}"))?;
    let core = context
        .connect_rc(None)
        .map_err(|error| format!("core: {error:?}"))?;

    let stats = Arc::new(Stats::default());
    let published = Arc::new(AtomicU64::new(0));

    let properties = pw::properties::properties! {
        *pw::keys::MEDIA_CLASS => "Video/Source",
        *pw::keys::MEDIA_ROLE => "Camera",
        *pw::keys::NODE_NAME => options.node_name,
        *pw::keys::NODE_DESCRIPTION => options.description,
        *pw::keys::NODE_VIRTUAL => "TRUE",
        *pw::keys::NODE_PAUSE_ON_IDLE => "FALSE",
        *pw::keys::PRIORITY_SESSION => "1000",
        *pw::keys::STREAM_IS_LIVE => "true",
    };

    let stream = pw::stream::StreamRc::new(core.clone(), options.node_name, properties)
        .map_err(|error| format!("stream: {error:?}"))?;

    let listener = stream
        .add_local_listener_with_user_data(UserData {
            format: Default::default(),
            feed: options.feed.clone(),
            stats: stats.clone(),
            published: published.clone(),
            quit: options.quit,
            failed: options.failed,
            logged_layout: false,
        })
        .state_changed(|_, data, old, new| {
            let streaming = matches!(&new, pw::stream::StreamState::Streaming);
            data.stats.streaming.store(streaming, Ordering::Relaxed);
            if matches!(&new, pw::stream::StreamState::Error(_)) {
                data.failed.store(true, Ordering::SeqCst);
                data.quit.store(true, Ordering::SeqCst);
            }
            eprintln!("publisher: state {old:?} -> {new:?}");
        })
        .param_changed(|stream, data, id, param| {
            if id != spa::param::ParamType::Format.as_raw() {
                return;
            }
            let Some(param) = param else {
                return;
            };
            let Ok((media_type, media_subtype)) = spa::param::format_utils::parse_format(param)
            else {
                return;
            };
            if media_type != MediaType::Video || media_subtype != MediaSubtype::Raw {
                return;
            }
            if data.format.parse(param).is_err() {
                return;
            }
            if data.format.format().as_raw() != VideoFormat::I420.as_raw() {
                eprintln!(
                    "publisher: negotiated non-I420 format {:?}",
                    data.format.format()
                );
                return;
            }

            let size = data.format.size();
            let buffers = buffers_param(size.width, size.height);
            let meta = meta_param();
            let mut params: [&Pod; 2] = [
                Pod::from_bytes(&buffers).expect("buffers pod"),
                Pod::from_bytes(&meta).expect("meta pod"),
            ];
            match stream.update_params(&mut params) {
                Ok(()) => eprintln!(
                    "publisher: buffers negotiated {}x{}",
                    size.width, size.height
                ),
                Err(error) => eprintln!("publisher: update_params failed: {error:?}"),
            }
        })
        .process(|stream, data| {
            data.stats.process_calls.fetch_add(1, Ordering::Relaxed);
            let raw = unsafe { stream.dequeue_raw_buffer() };
            if raw.is_null() {
                let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                data.stats.last_errno.store(code, Ordering::Relaxed);
                match code {
                    libc::EAGAIN => {
                        data.stats.eagain.fetch_add(1, Ordering::Relaxed);
                    }
                    libc::EBUSY => {
                        data.stats.ebusy.fetch_add(1, Ordering::Relaxed);
                    }
                    libc::EPIPE => {
                        data.stats.epipe.fetch_add(1, Ordering::Relaxed);
                    }
                    _ => {
                        data.stats.other_errno.fetch_add(1, Ordering::Relaxed);
                    }
                }
                return;
            }

            let item = match data.feed.try_lock() {
                Ok(mut feed) => feed.take_new().or_else(|| feed.latest()),
                Err(_) => None,
            };
            let frame = match item {
                Some((frame, _placeholder)) => frame,
                None => placeholder().clone(),
            };

            if !data.logged_layout {
                data.logged_layout = true;
                unsafe {
                    let buffer = (*raw).buffer;
                    if !buffer.is_null() {
                        eprintln!("publisher: layout n_datas={}", (*buffer).n_datas);
                    }
                }
            }

            fill_frame(raw, &frame, &data.stats);
            unsafe { stream.queue_raw_buffer(raw) };
            data.stats.frames.fetch_add(1, Ordering::Relaxed);
            data.published.fetch_add(1, Ordering::Relaxed);
        })
        .register()
        .map_err(|error| format!("listener: {error:?}"))?;

    let format = format_param();
    let mut params = [Pod::from_bytes(&format).expect("format pod")];
    stream
        .connect(
            spa::utils::Direction::Output,
            None,
            pw::stream::StreamFlags::DRIVER | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .map_err(|error| format!("connect: {error:?}"))?;

    let timer = {
        let mainloop_for_quit = mainloop.clone();
        let stream = stream.clone();
        let stats = stats.clone();
        let published = published.clone();
        let feed = options.feed.clone();
        let quit = options.quit;
        let debug_requested = options.debug_requested;
        let debug_until_ms = options.debug_until_ms;
        mainloop.loop_().add_timer(move |_| {
            if quit.load(Ordering::Relaxed) {
                mainloop_for_quit.quit();
                return;
            }
            let eof = feed.lock().map(|feed| feed.is_eof()).unwrap_or(false);
            if eof {
                mainloop_for_quit.quit();
                return;
            }
            if debug_requested.swap(false, Ordering::Relaxed) {
                debug_until_ms.store(monotonic_millis() + 20_000, Ordering::Relaxed);
            }

            if stats.streaming.load(Ordering::Relaxed)
                && stream.is_driving()
                && stream.trigger_process().is_err()
            {
                stats.trigger_errors.fetch_add(1, Ordering::Relaxed);
            }

            let tick = stats.timer_ticks.fetch_add(1, Ordering::Relaxed);
            if tick % 300 == 0 {
                let (received, placeholders) = feed
                    .lock()
                    .map(|feed| (feed.received(), feed.placeholders()))
                    .unwrap_or((0, 0));
                println!(
                    "publisher received={received}, placeholder={placeholders}, frames={}",
                    published.load(Ordering::Relaxed)
                );
            }
            if debug_until_ms.load(Ordering::Relaxed) > monotonic_millis() && tick % 30 == 0 {
                eprintln!(
                    "publisher: debug frames={} short={} eagain={} ebusy={} epipe={} other={}",
                    stats.frames.load(Ordering::Relaxed),
                    stats.short_buffers.load(Ordering::Relaxed),
                    stats.eagain.load(Ordering::Relaxed),
                    stats.ebusy.load(Ordering::Relaxed),
                    stats.epipe.load(Ordering::Relaxed),
                    stats.other_errno.load(Ordering::Relaxed),
                );
            }
        })
    };
    let _ = timer.update_timer(
        Some(Duration::from_nanos(FRAME_INTERVAL_NS)),
        Some(Duration::from_nanos(FRAME_INTERVAL_NS)),
    );

    println!(
        "publisher pid={}, plugin=native-pipewire",
        std::process::id()
    );
    mainloop.run();

    drop(timer);
    drop(listener);
    drop(stream);
    Ok(())
}
