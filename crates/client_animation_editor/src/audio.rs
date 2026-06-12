use bevy::audio::{AudioPlayer, AudioSource, PlaybackSettings};
use bevy::prelude::*;

use crate::state::{ClipSlot, EditorState, Playback};

/// Handle to the editor's event click sfx, loaded at startup.
#[derive(Resource)]
pub struct EventClickAudio(pub Handle<AudioSource>);

/// Loads the click played when the playhead crosses an animation event.
pub fn load_event_click_audio(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(EventClickAudio(
        asset_server.load("audio/editor_event_click.ogg"),
    ));
}

/// Plays a click whenever the advancing playhead crosses an event time.
///
/// Bevy's `AnimationEventFired` can never fire in the editor: the transport drives the
/// selected nodes at speed 0 and `set_seek_time`, so advance normalizes the
/// (last_seek, seek) pair before the trigger system reads it. Crossings are therefore
/// detected editor-side from the playhead. Gating falls out of the comparison: only a
/// `Playing` advance against the remembered playhead fires; pausing, scrubbing, or
/// switching clips just resyncs, so scrubs are silent by construction.
pub fn play_event_audio(
    state: Res<EditorState>,
    click: Res<EventClickAudio>,
    mut commands: Commands,
    mut last: Local<Option<(ClipSlot, f32)>>,
) {
    if let Some((slot, last_t)) = last.as_ref() {
        if *slot == state.selected_clip && state.playback == Playback::Playing {
            for event in &state.working.events {
                if playhead_crossed(*last_t, state.playhead, event.time) {
                    commands.spawn((AudioPlayer(click.0.clone()), PlaybackSettings::DESPAWN));
                }
            }
        }
    }
    *last = Some((state.selected_clip.clone(), state.playhead));
}

/// Forward crossing test matching Bevy's event windows: `[last, current)` normally; on a
/// wrap (`current < last`), `t >= last` or `t < current` — so an event at exactly
/// `duration` fires at the wrap (tail slice) and one at `t = 0` fires entering the new
/// loop (head slice), each exactly once per loop.
fn playhead_crossed(last: f32, current: f32, t: f32) -> bool {
    if current >= last {
        last <= t && t < current
    } else {
        t >= last || t < current
    }
}

#[cfg(test)]
mod tests {
    use super::playhead_crossed;

    #[test]
    fn forward_window_is_half_open() {
        assert!(playhead_crossed(0.2, 0.4, 0.2));
        assert!(playhead_crossed(0.2, 0.4, 0.3));
        assert!(!playhead_crossed(0.2, 0.4, 0.4));
        assert!(!playhead_crossed(0.2, 0.4, 0.1));
    }

    #[test]
    fn wrap_fires_tail_and_head_once() {
        // last=0.9 → current=0.1 across a 1.0s wrap.
        assert!(playhead_crossed(0.9, 0.1, 1.0)); // event at duration: tail slice
        assert!(playhead_crossed(0.9, 0.1, 0.0)); // event at zero: head slice
        assert!(playhead_crossed(0.9, 0.1, 0.95));
        assert!(!playhead_crossed(0.9, 0.1, 0.5));
        // The same events do not re-fire on the following normal tick.
        assert!(!playhead_crossed(0.1, 0.3, 1.0));
        assert!(!playhead_crossed(0.1, 0.3, 0.0));
    }
}
