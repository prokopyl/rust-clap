use clap_audio_common::events::{
    event_match::EventTarget, event_types::NoteEvent, list::EventList, Event, EventType,
};

use clap_audio_host::{
    entry::PluginEntry,
    host::{HostInfo, PluginHost},
    instance::processor::audio::HostAudioBufferCollection,
    instance::PluginAudioConfiguration,
};
use std::cell::RefCell;

use gain::clap_plugin_entry;

#[test]
pub fn it_works() {
    // Initialize host with basic info
    let host = PluginHost::new(HostInfo::new("test", "", "", "").unwrap());

    // Get plugin entry from the exported static
    // SAFETY: only called this once here
    let entry = unsafe { PluginEntry::from_descriptor(&clap_plugin_entry, "") }.unwrap();
    let desc = entry.plugin_descriptor(0).unwrap();
    assert_eq!(desc.id().unwrap(), "gain");

    // Instantiate the desired plugin
    // Using RefCell is dumb but enough for single-threaded testing
    let plugin = RefCell::new(entry.instantiate("gain", &host));

    // Setting up some buffers
    let configuration = PluginAudioConfiguration {
        sample_rate: 44_100.0,
        frames_count_range: 32..=32,
    };
    let inputs = HostAudioBufferCollection::for_ports_and_channels(1, 2, || vec![69f32; 32]);
    let mut outputs = HostAudioBufferCollection::for_ports_and_channels(1, 2, || vec![0f32; 32]);

    let event = Event::new(
        1,
        EventType::NoteOn(NoteEvent::new(
            42.into(),
            EventTarget::All,
            EventTarget::All,
            6.9,
        )),
    );
    let mut event_buffer_in = vec![event; 32];
    let mut event_buffer_out = vec![];

    let mut events_in = EventList::from_implementation(&mut event_buffer_in);
    let mut events_out = EventList::from_implementation(&mut event_buffer_out);

    let mut processor = plugin
        .borrow_mut()
        .activate(configuration, |msg| {
            // Technically that's an spsc "channel" ¯\_(ツ)_/¯
            plugin.borrow_mut().process_received_message(msg)
        })
        .unwrap()
        .start_processing()
        .unwrap();

    // Process
    processor.process(&inputs, &mut outputs, &mut events_in, &mut events_out);

    // Check the gain was applied properly
    for channel_index in 0..1 {
        let inbuf = inputs.get_channel_buffer(0, channel_index).unwrap();
        let outbuf = outputs.get_channel_buffer(0, channel_index).unwrap();
        for (input, output) in inbuf.iter().zip(outbuf.iter()) {
            assert_eq!(*output, *input * 2.0)
        }
    }

    // Check velocity was changed properly
    assert_eq!(event_buffer_in.len(), event_buffer_out.len());

    for (input, output) in event_buffer_in.iter().zip(event_buffer_out.iter()) {
        let input_note = if let Some(EventType::NoteOn(ev)) = input.event() {
            ev
        } else {
            panic!("Invalid event type found")
        };

        assert_eq!(
            output,
            &Event::new(
                input.time(),
                EventType::NoteOn(NoteEvent::new(
                    42.into(),
                    EventTarget::All,
                    EventTarget::All,
                    input_note.velocity() * 2.0
                ))
            )
        )
    }
}
