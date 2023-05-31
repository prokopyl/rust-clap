use crate::stream::activate_to_stream;
use clack_extensions::audio_ports::{
    AudioPortInfoBuffer, HostAudioPortsImpl, PluginAudioPorts, RescanType,
};
use clack_extensions::audio_ports_config::PluginAudioPortsConfig;
use clack_extensions::gui::{
    GuiApiType, GuiError, GuiSize, HostGui, HostGuiImpl, PluginGui, Window as ClapWindow,
};
use clack_extensions::log::{HostLog, HostLogImpl, LogSeverity};
use clack_extensions::params::{
    HostParams, HostParamsImplMainThread, HostParamsImplShared, ParamClearFlags, ParamRescanFlags,
};
use clack_extensions::timer::{HostTimer, HostTimerImpl, PluginTimer, TimerError, TimerId};
use clack_host::prelude::*;
use cpal::traits::StreamTrait;
use crossbeam_channel::{unbounded, Receiver, Sender};
use std::collections::HashMap;
use std::error::Error;
use std::ffi::CString;
use std::path::Path;
use std::time::{Duration, Instant};
use winit::dpi::PhysicalSize;
use winit::event::{Event, WindowEvent};
use winit::event_loop::{EventLoopBuilder, EventLoopWindowTarget};
use winit::window::{Window, WindowBuilder};

pub struct CpalHost;
pub struct CpalHostShared<'a> {
    sender: Sender<MainThreadMessage>,
    plugin: Option<PluginSharedHandle<'a>>,
    gui: Option<&'a PluginGui>,
    audio_ports: Option<&'a PluginAudioPorts>,
    audio_ports_config: Option<&'a PluginAudioPortsConfig>,
}

impl<'a> CpalHostShared<'a> {
    fn new(sender: Sender<MainThreadMessage>) -> Self {
        Self {
            sender,
            plugin: None,
            gui: None,
            audio_ports: None,
            audio_ports_config: None,
        }
    }
}

impl<'a> HostLogImpl for CpalHostShared<'a> {
    fn log(&self, severity: LogSeverity, message: &str) {
        if severity.to_raw() <= LogSeverity::Debug.to_raw() {
            return;
        };
        eprintln!("[{severity}] {message}")
    }
}

impl<'a> HostAudioPortsImpl for CpalHostMainThread<'a> {
    fn is_rescan_flag_supported(&self, flag: RescanType) -> bool {
        true
    }

    fn rescan(&mut self, flag: RescanType) {
        todo!()
    }
}

enum MainThreadMessage {
    RunOnMainThread,
    GuiClosed { was_destroyed: bool },
    WindowClosing,
    Tick,
}

impl<'a> HostShared<'a> for CpalHostShared<'a> {
    fn instantiated(&mut self, instance: PluginSharedHandle<'a>) {
        self.gui = instance.get_extension();
        self.audio_ports = instance.get_extension();
        self.plugin = Some(instance);
    }

    fn request_restart(&self) {
        todo!()
    }

    fn request_process(&self) {
        // We never pause, and CPAL is in full control anyway
    }

    fn request_callback(&self) {
        self.sender
            .send(MainThreadMessage::RunOnMainThread)
            .unwrap();
    }
}

pub struct CpalHostMainThread<'a> {
    shared: &'a CpalHostShared<'a>,
    pub plugin: Option<PluginMainThreadHandle<'a>>,
    pub available_gui_api: Option<(CString, bool)>,
    timer_support: Option<&'a PluginTimer>,
    timers: Timers,
    gui_open: bool,
}

impl<'a> CpalHostMainThread<'a> {
    fn new(shared: &'a CpalHostShared) -> Self {
        Self {
            shared,
            plugin: None,
            available_gui_api: None,
            timer_support: None,
            timers: Timers::new(),
            gui_open: false,
        }
    }

    fn gui_can_float(&self) -> Option<bool> {
        self.available_gui_api.as_ref().map(|(_, float)| (*float))
    }

    fn open_embedding_window(
        &mut self,
        event_loop: &EventLoopWindowTarget<MainThreadMessage>,
    ) -> Result<Window, Box<dyn Error>> {
        let gui = self.shared.gui.unwrap();
        let (api, _) = self.available_gui_api.as_ref().unwrap();
        let plugin = self.plugin.as_mut().unwrap();
        gui.create(plugin, GuiApiType(api), false)?;
        let initial_size = gui.get_size(plugin).unwrap_or(GuiSize {
            width: 640,
            height: 480,
        });

        // TODO: resizeable & stuff
        // let resizeable = gui.can_resize(plugin);

        let window = WindowBuilder::new()
            .with_title("Clack CPAL plugin!")
            .with_inner_size(PhysicalSize {
                height: initial_size.height,
                width: initial_size.width,
            })
            .build(event_loop)?;

        gui.set_parent(plugin, &ClapWindow::from_window(&window).unwrap())?;
        gui.show(plugin)?;
        self.gui_open = true;

        Ok(window)
    }

    fn destroy_gui(&mut self) {
        if !self.gui_open {
            return;
        }
        let gui = self.shared.gui.unwrap();
        let plugin = self.plugin.as_mut().unwrap();

        gui.destroy(plugin);
        self.gui_open = false;
    }

    fn tick_timers(&mut self) {
        let Some(timer) = self.timer_support else { return };
        let plugin = self.plugin.as_mut().unwrap();

        for triggered in self.timers.tick_all() {
            timer.on_timer(plugin, triggered);
        }
    }
}

impl<'a> HostMainThread<'a> for CpalHostMainThread<'a> {
    fn instantiated(&mut self, instance: PluginMainThreadHandle<'a>) {
        if let Some(gui) = self.shared.gui {
            self.available_gui_api = gui
                .get_preferred_api(&instance)
                .map(|(ty, floating)| {
                    if floating {
                        return (ty, floating);
                    }
                    (ty, gui.is_api_supported(&instance, ty, true))
                })
                .or_else(|| {
                    let platform = GuiApiType::default_for_current_platform()?;
                    if gui.is_api_supported(&instance, platform, true) {
                        Some((platform, true))
                    } else if gui.is_api_supported(&instance, platform, false) {
                        Some((platform, false))
                    } else {
                        None
                    }
                })
                .map(|(api, floating)| (api.0.to_owned(), floating));
        }

        self.timer_support = instance.shared().get_extension();
        self.plugin = Some(instance);
    }
}

impl<'a> HostTimerImpl for CpalHostMainThread<'a> {
    fn register_timer(&mut self, period_ms: u32) -> Result<TimerId, TimerError> {
        Ok(self.timers.register_new(period_ms))
    }

    fn unregister_timer(&mut self, timer_id: TimerId) -> Result<(), TimerError> {
        if self.timers.unregister(timer_id) {
            Ok(())
        } else {
            Err(TimerError::UnregisterError)
        }
    }
}

impl<'a> HostParamsImplMainThread for CpalHostMainThread<'a> {
    fn rescan(&mut self, flags: ParamRescanFlags) {
        // todo!()
    }

    fn clear(&mut self, param_id: u32, flags: ParamClearFlags) {
        todo!()
    }
}

impl<'a> HostParamsImplShared for CpalHostShared<'a> {
    fn request_flush(&self) {
        todo!()
    }
}

impl<'a> HostGuiImpl for CpalHostShared<'a> {
    fn resize_hints_changed(&self) {
        // todo!()
    }

    fn request_resize(&self, _new_size: GuiSize) -> Result<(), GuiError> {
        todo!()
    }

    fn request_show(&self) -> Result<(), GuiError> {
        todo!()
    }

    fn request_hide(&self) -> Result<(), GuiError> {
        todo!()
    }

    fn closed(&self, was_destroyed: bool) {
        self.sender
            .send(MainThreadMessage::GuiClosed { was_destroyed })
            .unwrap();
    }
}

impl Host for CpalHost {
    type Shared<'a> = CpalHostShared<'a>;
    type MainThread<'a> = CpalHostMainThread<'a>;
    type AudioProcessor<'a> = ();

    fn declare_extensions(builder: &mut HostExtensions<Self>, _shared: &Self::Shared<'_>) {
        builder
            .register::<HostLog>()
            .register::<HostGui>()
            .register::<HostTimer>()
            .register::<HostParams>();
    }
}

pub fn run(bundle_path: &Path, plugin_id: &str) -> Result<(), Box<dyn Error>> {
    let bundle = PluginBundle::load(bundle_path)?;

    let host_info = host_info();
    let plugin_id = CString::new(plugin_id)?;
    let (sender, receiver) = unbounded();

    let mut instance = PluginInstance::<CpalHost>::new(
        |_| CpalHostShared::new(sender.clone()),
        |shared| CpalHostMainThread::new(shared),
        &bundle,
        &plugin_id,
        &host_info,
    )?;

    AudioPortsConfig::from_plugin(
        instance.main_thread_host_data().plugin.as_ref().unwrap(),
        instance.shared_host_data().audio_ports,
    );

    let run_ui = match instance.main_thread_host_data().gui_can_float() {
        Some(true) => run_gui_floating,
        Some(false) => run_gui_embedded,
        None => run_cli,
    };

    let stream = activate_to_stream(&mut instance)?;

    run_ui(instance, receiver)?;

    stream.pause()?;

    Ok(())
}

fn run_gui_floating(
    mut instance: PluginInstance<CpalHost>,
    receiver: Receiver<MainThreadMessage>,
) -> Result<(), Box<dyn Error>> {
    let main_thread = instance.main_thread_host_data_mut();
    let (api, _) = main_thread.available_gui_api.as_ref().unwrap();
    println!("Opening GUI type: {api:?} in floating mode");
    let gui = main_thread.shared.gui.unwrap();
    let plugin = main_thread.plugin.as_mut().unwrap();

    gui.create(plugin, GuiApiType(api), false)?;
    gui.show(plugin)?;

    for message in receiver {
        match message {
            MainThreadMessage::RunOnMainThread => instance.call_on_main_thread_callback(),
            MainThreadMessage::Tick => instance.main_thread_host_data_mut().tick_timers(),
            MainThreadMessage::GuiClosed { was_destroyed } => {
                println!("Window closed!");
                break;
            }
            _ => {}
        }
    }

    instance.main_thread_host_data_mut().destroy_gui();

    Ok(())
}

fn run_gui_embedded(
    mut instance: PluginInstance<CpalHost>,
    receiver: Receiver<MainThreadMessage>,
) -> Result<(), Box<dyn Error>> {
    let main_thread = instance.main_thread_host_data_mut();
    let (api, _) = main_thread.available_gui_api.as_ref().unwrap();
    println!("Opening GUI type: {api:?} in embedded mode");

    let event_loop = EventLoopBuilder::with_user_event().build();

    // TODO: handle events
    let mut window = Some(main_thread.open_embedding_window(&event_loop)?);

    event_loop.run(move |event, target, control_flow| {
        while let Ok(message) = receiver.try_recv() {
            match message {
                MainThreadMessage::RunOnMainThread => instance.call_on_main_thread_callback(),
                MainThreadMessage::WindowClosing => {
                    println!("Window closed!");
                    break;
                }
                _ => {}
            }
        }

        match event {
            Event::WindowEvent { event, window_id } => match event {
                WindowEvent::CloseRequested | WindowEvent::Destroyed => {
                    println!("Received close {window_id:?}");
                    instance.main_thread_host_data_mut().destroy_gui();
                    window.take(); // Drop the window
                    control_flow.set_exit();
                    return;
                }
                _ => {}
            },
            Event::LoopDestroyed => {
                instance.main_thread_host_data_mut().destroy_gui();
            }
            _ => {}
        }

        let main_thread = instance.main_thread_host_data_mut();
        main_thread.tick_timers();
        control_flow.set_wait_timeout(
            main_thread
                .timers
                .smallest_duration()
                .unwrap_or(Duration::from_millis(60)),
        );
    });
}

fn run_cli(
    mut instance: PluginInstance<CpalHost>,
    receiver: Receiver<MainThreadMessage>,
) -> Result<(), Box<dyn Error>> {
    println!("Running headless. Press Ctrl+C to stop processing.");

    for message in receiver {
        if let MainThreadMessage::RunOnMainThread = message {
            instance.call_on_main_thread_callback()
        }
    }

    Ok(())
}

//fn process(audio_processor: StartedPluginAudioProcessor<CpalHost>, data) {

//}

fn host_info() -> HostInfo {
    HostInfo::new(
        "Clack example CPAL host",
        "Clack",
        "https://github.com/prokopyl/clack",
        "0.0.0",
    )
    .unwrap()
}

struct AudioPortsConfig {
    input_channel_counts: Vec<usize>,
    output_channel_counts: Vec<usize>,
}

impl AudioPortsConfig {
    fn from_plugin(handle: &PluginMainThreadHandle, ports: Option<&PluginAudioPorts>) -> Self {
        println!("Scanning plugin ports:");
        let Some(ports) = ports else {
            println!("No ports extension available: assuming single stereo port for input and output");
            return Self {
                input_channel_counts: vec![2],
                output_channel_counts: vec![2],
            }
        };

        let input_channel_counts = vec![];
        let mut buf = AudioPortInfoBuffer::new();
        let count = ports.count(handle, true);

        for i in 0..count {
            let config = ports.get(handle, i, true, &mut buf).unwrap();
            println!("config: {config:?}");
        }
        let count = ports.count(handle, false);
        for i in 0..count {
            let config = ports.get(handle, i, false, &mut buf).unwrap();
            println!("config: {config:?}");
        }

        Self {
            input_channel_counts,
            output_channel_counts: vec![],
        }
    }
}

struct Timers {
    latest_id: u32,
    smallest_duration: Option<u32>,
    timers: HashMap<TimerId, Timer>,
}

impl Timers {
    fn new() -> Self {
        Self {
            latest_id: 0,
            timers: HashMap::new(),
            smallest_duration: None,
        }
    }

    fn tick_all(&mut self) -> impl Iterator<Item = TimerId> + '_ {
        let now = Instant::now();

        self.timers
            .values_mut()
            .filter_map(move |t| t.tick(now).then_some(t.id))
    }

    fn register_new(&mut self, interval: u32) -> TimerId {
        self.latest_id += 1;
        let id = TimerId(self.latest_id);
        self.timers.insert(id, Timer::new(id, interval));

        match self.smallest_duration {
            None => self.smallest_duration = Some(interval),
            Some(smallest) if smallest > interval => self.smallest_duration = Some(interval),
            _ => {}
        }

        id
    }

    fn unregister(&mut self, id: TimerId) -> bool {
        if self.timers.remove(&id).is_some() {
            self.smallest_duration = self.timers.values().map(|t| t.interval).min();
            true
        } else {
            false
        }
    }

    fn smallest_duration(&self) -> Option<Duration> {
        self.smallest_duration
            .map(|i| Duration::from_millis(i as u64))
    }
}

struct Timer {
    id: TimerId,
    interval: u32,
    last_updated_at: Option<Instant>,
}

impl Timer {
    fn new(id: TimerId, interval: u32) -> Self {
        Self {
            id,
            interval,
            last_updated_at: None,
        }
    }

    fn tick(&mut self, now: Instant) -> bool {
        let triggered = if let Some(last_updated_at) = self.last_updated_at {
            if let Some(since) = now.checked_duration_since(last_updated_at) {
                since > Duration::from_millis(self.interval as u64)
            } else {
                false
            }
        } else {
            true
        };
        self.last_updated_at = Some(now);

        triggered
    }
}