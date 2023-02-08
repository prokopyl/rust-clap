//! The host side of the Clack extension system.
//!
//! The majority of the features in CLAP come from extensions, including
//! parameters management, state loading and saving, GUI handling, and more.
//!
//! The goal of this system is to maximize flexibility and extensibility, while also preventing
//! feature creep by not requiring all hosts to implement every single extension. This module
//! provides thin wrappers that maintain those abilities while also enforcing both type safety and
//! memory safety.
//!
//! This crate does not include any extension itself. All official, first-party CLAP extensions are
//! implemented on top of it in the `clack-extensions` crate, but all extension implementations are
//! treated as first-class citizens by both CLAP and Clack, regardless of their provenance. See the
//! [Creating custom extensions](#creating-custom-extensions) section below for more information on
//! how to implement third-party extensions.
//!
//! At instantiation time, both the plugin and the host will query each other's declared supported
//! extensions. When one side declares supporting an extension, it will also provide an [`Extension`]
//! object containing that extension's ABI to the other side. See the [`Extension`] type
//! documentation for more information on how to use and store them.
//!
//! [`Extension`] ABIs are split in two parts: one that is exposed by the host, and one that is
//! exposed by the plugin. For instance, for the `Latency` extension, the ABIs are named `HostLatency`
//! and `PluginLatency` respectively.
//!
//! # Using extensions in a Clack host
//!
//! Supporting a specific extension in a CLAP host has two requirements:
//!
//! * Querying a plugin for its side of the ABI, and consuming it.
//!   
//!   This is the most straightforward part: once the plugin is instantiated and the host can access
//!   its [`PluginSharedHandle`](crate::instance::handle::PluginSharedHandle), it can use the
//!   [`PluginSharedHandle::get_extension`](crate::instance::handle::PluginSharedHandle::get_extension)
//!   method to query the plugin for any supported extension, and store its associated ABI.
//!
//!   References to an Extension ABI can be shared, copied and used in any thread as long as they
//!   don't outlive the plugin instance. They are therefore most commonly stored in the host's
//!   [`HostShared`](crate::host::HostShared) associated type, as shown in the example below.
//!
//!
//! * Implementing the host side of the ABI, and exposing it to the plugin to be queried.
//!
//!   All extensions in Clack have at least one trait to be implemented onto a specific
//!   [`Host`](crate::host::Host) subtype ([`HostMainThread`](crate::host::HostMainThread),
//!   [`HostAudioProcessor`](crate::host::HostAudioProcessor),
//!   or [`HostShared`](crate::host::HostShared)), depending on the thread specification of the
//!   ABI's method. For example, the `Log` extension's ABI has to be fully thread-safe, therefore
//!   the `HostLogImpl` trait has to be implemented on the [`HostShared`](crate::host::HostShared)
//!   type.
//!
//!   See the [`host`](crate::host) module documentation to know more about CLAP's
//!   thread specification.
//!   
//!   Sometimes however, some ABIs expose different methods in different thread classes, leading
//!   to that many traits to be implemented on different types. For instance, the `Params` ABI
//!   exposes one thread-safe method and two that are main-thread only. Therefore, the
//!   `HostParamsImplShared` and `HostParamsImplMainThread` traits have to be implemented on the
//!   [`HostShared`](crate::host::HostShared) and [`HostMainThread`](crate::host::HostMainThread)
//!   types, respectively.
//!
//!   Once this is all done, the host implementation can declare this extension by using the
//!   [`HostExtensions::register`](crate::host::HostExtensions::register) method in the
//!   [`Host::declare_extensions`](crate::host::Host::declare_extensions) method implementation.
//!
//!   The fact that the right traits are implemented on the right [`Host`](crate::host::Host)
//!   associated types is automatically checked at compile time, upon calling the
//!   [`HostExtensions::register`](crate::host::HostExtensions::register) method.
//!
//! ## Example
//!
//! This example implements a host supporting the `Latency` extension.
//!
//! ```
//! use clack_host::prelude::*;
//! use clack_extensions::latency::*;
//!
//! #[derive(Default)]
//! struct MyHostShared<'a> {
//!     // Queried extension
//!     // Note this may be None even after instantiation,
//!     // in case the extension isn't supported by the plugin.
//!     latency_extension: Option<&'a PluginLatency>
//! }
//!
//! impl<'a> HostShared<'a> for MyHostShared<'a> {
//!     // Once the plugin is fully instantiated, we can query its extensions
//!     fn instantiated(&mut self, instance: PluginSharedHandle<'a>) {
//!         self.latency_extension = instance.get_extension();
//!     }
//!     
//!     /* ... */
//!     # fn request_restart(&self) { unimplemented!() }
//!     # fn request_process(&self) { unimplemented!() }
//!     # fn request_callback(&self) { unimplemented!() }
//! }
//!
//! struct MyHostMainThread<'a> {
//!     shared: &'a MyHostShared<'a>,
//!     instance: Option<PluginMainThreadHandle<'a>>,
//!
//!     // The latency that is sent to us by the plugin's Latency extension.
//!     reported_latency: Option<u32>
//! }
//!
//! impl<'a> HostMainThread<'a> for MyHostMainThread<'a> {
//!     // The plugin's instance handle is required to call extension methods.
//!     fn instantiated(&mut self, instance: PluginMainThreadHandle<'a>) {
//!         self.instance = Some(instance);
//!     }
//! }
//!
//! impl<'a> HostLatencyImpl for MyHostMainThread<'a> {
//!     // This method is called by the plugin whenever its latency changed.
//!     fn changed(&mut self) {
//!         // Ensure that the plugin is instantiated and supports the Latency extension.
//!         if let (Some(latency), Some(instance)) = (self.shared.latency_extension, &mut self.instance) {
//!             self.reported_latency = Some(latency.get(instance));
//!         }   
//!     }
//! }
//!
//! struct MyHost;
//! impl<'a> Host<'a> for MyHost {
//!     type Shared = MyHostShared<'a>;
//!
//!     type MainThread = MyHostMainThread<'a>;
//!     type AudioProcessor = ();
//!
//!     fn declare_extensions(builder: &mut HostExtensions<Self>, shared: &Self::Shared) {
//!         builder.register::<HostLatency>();
//!     }
//! }
//! ```
//!
//! # Creating custom extensions
//!
//! TODO: document custom extensions.

pub use clack_common::extensions::*;
pub mod wrapper;
