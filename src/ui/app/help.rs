//! AMP owns the command catalogue; KIT owns browsing and presentation.
use super::keymap::{self, Action};
use starkit::help::{Command, Guide};

fn category(group: &str) -> &'static str {
    match group {
        "transport" | "progress bar" => "Playback",
        "playlist" | "tagging" => "Playlist",
        "library" => "Library",
        "equalizer" => "Equalizer",
        "album" => "Artwork",
        "visualizer" | "appearance" => "Appearance",
        _ => "Workspace",
    }
}
pub fn guide() -> Guide {
    let mut commands: Vec<_> = keymap::BINDINGS.iter().enumerate().map(|(i,b)| {
        let context = match b.group {
            "equalizer" => "Focus the Equalizer with Alt+3. Its arrows select filters and adjust gain; Enter bypasses a filter. Closing the panel does not bypass the EQ.",
            "album" => "Focus Album with Alt+4. Left/Right cycle images; Enter opens the original image externally. Album keys take precedence over global player keys.",
            "library" => "Open the library with L (lowercase). Here Space adds the selection and A (lowercase) adds its album; arrows change columns. These keys apply while browsing the library.",
            "tagging" => "Use this in the playlist. Tags select queue rows; they do not edit the audio file's metadata. Removing rows changes the queue and never deletes your music files.",
            "playlist" => "Focus the Playlist with Alt+2. Arrow keys and J/K move the cursor; Enter plays the selected track. Sorting and filtering affect the playlist view.",
            "visualizer" => "The visualizer belongs to the player panel. W cycles forward, Shift+W cycles backward; + and - change bar width. Playback continues while changing its appearance.",
            _ => "Use this from the main player, outside a text field or modal dialog. Some keys have a different meaning in the focused panel; Alt+1 focuses the player.",
        };
        let (title, details, aliases) = match b.action {
            Action::ToggleHistoryPanel => ("Show listening history and Activity", "Press Alt+S to show or hide Activity. It contains local listening history and the Last.fm and ListenBrainz providers. Closing this panel does not stop scrobbling. Use its settings to configure providers.", "history activity scrobbler scrobbling listened recent logs lastfm listenbrainz"),
            Action::ToggleEqPanel => ("Show or hide the equalizer", "Alt+G toggles the Equalizer panel. Alt+3 focuses it and opens it if needed. Closing the panel keeps the sound processing active; E toggles processing.", "eq audio filter tone bass treble panel"),
            Action::ToggleAlbumPanel => ("Show or hide album information", "Press I to toggle Album. Alt+4 focuses it. The panel contains cover artwork, artist, album and release information.", "artwork cover metadata information window"),
            Action::TogglePlaylistPanel => ("Show or hide the playlist", "Press Alt+P to toggle the queue panel. Alt+2 focuses it and opens it if necessary. Playback continues when the panel is hidden.", "queue tracks window collapse expand"),
            Action::FocusPlayer => ("Focus a player panel", "Alt+1 Player; Alt+2 Playlist; Alt+3 Equalizer; Alt+4 Album; Alt+5 Activity. Focusing a hidden panel opens it. Tab and Shift+Tab cycle visible panels.", "keyboard focus navigation windows panes"),
            Action::PlayPause => ("Play or pause", "Press Space, c or x from the player to toggle playback. The single Play/Pause button shows the action it will perform. In the library, Space adds the selected item instead; focus Player with Alt+1 for playback keys.", "transport resume start stop music playback toggle"),
            Action::PrevVisualizer => ("Previous visualization", context, "analyzer spectrum oscilloscope visualizer visualisation bars leds"),
            Action::ToggleVisualizer => ("Next visualization", context, "analyzer spectrum oscilloscope visualizer visualisation bars leds"),
            Action::OpenFilter => ("Sort or order the playlist", "Press F or click Sorting in the playlist header. Choose the ordering you want in the picker. Use / to search/filter the queue instead.", "sort order arrange artist album year timeline"),
            Action::FilterQueue => ("Search or filter the playlist", "Press / to search the current queue. Type the text or query, use arrows to choose a result, and Escape to close the search. This is separate from browsing your full library with L (lowercase).", "find search filter queue tracks artist song album"),
            Action::LoadLibrary => ("Load the whole music library", "Press Shift+L to replace the current queue with the full indexed library. Press L (lowercase) to browse without loading everything.", "all tracks collection reset library queue"),
            Action::LibrarySearch => ("Search the library", "Open the library with L (lowercase), then press /. Type your search in its search line. Library navigation keys take precedence while the browser is open.", "find artist song album collection query"),
            Action::NextTheme => ("Cycle the colour theme", "Press Alt+T to cycle the available themes. The theme name appears in the footer. The graphical rack and cell interface share KIT's theme palettes.", "color colour palette appearance dark light theme"),
            Action::Help => ("Search the command guide", "Type words in any order. Search includes shortcuts, mouse gestures, aliases and instructions, and tolerates common spelling mistakes. Use quotes for exact phrases and -word to exclude a word. Narrow results with category:audio, key:ctrl+s or title:record. Tab changes category; Ctrl+F searches all categories; Ctrl+U clears. Scroll instructions or use Ctrl+PageUp/PageDown. Esc or F1 closes help.", "manual documentation tutorial find command help search filter"),
            Action::ClearTags => ("Clear all tagged playlist rows", context, "selection unmark clear tracks tags"),
            Action::CopyTagged => ("Copy tagged tracks", context, "yank clipboard selected copy queue"),
            Action::RemoveTagged => ("Remove tagged tracks from the queue", context, "delete remove erase queue files kept"),
            Action::MoveTagged => ("Move tagged tracks to the cursor", context, "reorder rearrange selection move queue"),
            Action::LibraryAddAlbum => ("Add the selected album to the queue", context, "record enqueue library"),
            Action::LibraryAdd => ("Add the library selection to the queue", context, "enqueue music tracks"),
            Action::EqRemoveStage => ("Remove the selected EQ filter", context, "equalizer stage delete"),
            Action::EqToggleStage => ("Bypass the selected EQ filter", context, "disable enable equalizer stage"),
            Action::ToggleEqEnabled => ("Enable or bypass equalizer processing", context, "eq audio dsp sound bypass enable disable"),
            Action::OpenCover => ("Open the full cover image", context, "artwork image zoom album"),
            Action::PrevCover => ("Previous cover image", context, "artwork image album"),
            Action::NextCover => ("Next cover image", context, "artwork image album"),
            Action::ChooseCover => ("Choose album cover artwork", "Press Alt+I, click Choose in Album, or focus Album and press C. Select an available artwork source in the chooser.", "artwork art image cover picture select"),
            Action::RetryCover => ("Look up missing album artwork", "Press Alt+R or click Retry in Album. Artwork lookup uses the album's metadata and available cover sources. It runs separately from optional scrobbling services.", "refresh missing cover art picture fetch lookup download"),
            _ => (b.label, context, b.group),
        };
        let shortcut = match b.action {
            Action::PlayPause => "space / c / x",
            Action::FocusPlayer => "Alt+1…5",
            _ => b.keys,
        };
        let mut chars=title.chars();
        let title=chars.next().map(|c|c.to_uppercase().collect::<String>()+chars.as_str()).unwrap_or_default();
        Command::new(format!("key:{i}"),title,category(b.group)).shortcut(shortcut).summary(format!("{} · {}",category(b.group),b.label)).instructions(details).keywords(aliases)
    }).collect();
    commands.extend(keymap::MOUSE.iter().enumerate().map(|(i,m)| {
        let title = format!("{}: {}",m.group,m.label);
        Command::new(format!("mouse:{i}"),title,category(m.group)).shortcut(m.gesture)
            .summary(format!("Mouse gesture in {}",m.group))
            .instructions(format!("In the {} area: {} to {}. Focus the panel first if it is inactive. Mouse gestures act on the control under the pointer.",m.group,m.gesture,m.label))
            .keywords(format!("mouse pointer gesture {} {}",m.group,m.label))
    }));
    for (id,title,group,key,summary,instructions,aliases) in [
        ("eq-frequency","Edit filter frequency, gain and width","Equalizer","","Set precise filter parameters.","Focus Equalizer with Alt+3 and select a filter. Open its Settings to edit frequency in Hz, gain in dB, and Q / bandwidth / slope. Filter type and channel routing are also available there. Arrow keys provide quick gain changes.","parametric equalizer hz db q bandwidth slope channels left right bass treble"),
        ("eq-order","Reorder or duplicate EQ stages","Equalizer","","Change the ordered processing chain.","Select a filter in Equalizer, then open its Settings. Choose Duplicate selected stage, Move selected stage up, or Move selected stage down. Processing follows the displayed order.","chain order routing duplicate filter stage dsp"),
        ("eq-import","Import or export an EQ profile","Equalizer","","Share profiles using Equalizer APO files.","Open Equalizer Settings and choose Import EQ profile or Export APO profile. Enter the file path in the prompt. Imported profiles can be saved and managed in the profile library.","apo presets file load save export import eq profile"),
        ("discord","Show your current track in Discord","Activity & services","","Enable optional Discord Rich Presence.","Open Activity with Alt+S and choose its Settings, then Discord presence. Configuration needs a Discord application ID named STAR/AMP in the discord configuration section. The local Discord client must be running. This service is independent of local history and scrobbling.","discord presence rich status playing music account app id"),
        ("audio-compatibility","Set SSH audio compatibility","Playback","","Choose explicit compatibility conversion for remote audio.","Open Player Settings and choose SSH audio compatibility. Conversion is explicit; strict playback does not silently move sound to the host when a frontend output fails. The playback footer reports the active audio route.","remote output sound device sample rate conversion pcm bitperfect bit perfect ssh"),
        ("eq-profiles","Manage equalizer profiles","Equalizer","","Load, save or create a named processing profile.","Open the Equalizer profile library from its settings/profile controls. Type to search; Enter loads; Ctrl+S saves; Ctrl+N creates; Ctrl+R renames; Delete removes a profile. Escape closes it. Confirmations are required before deleting or reverting profiles.","presets equalizer APO import export save rename filter dsp"),
        ("settings","Open panel settings","Workspace","Click Settings","Configure the function associated with each panel.","Click Settings in the header of Player, Album, Equalizer, Playlist or Activity. Settings belong to that panel. Use arrows and Enter to change a value; Escape returns to the player.","preferences config configuration options output device audio"),
        ("activity-settings","Configure Last.fm and ListenBrainz","Activity & services","","Manage optional listening-history providers.","Open Activity with Alt+S, then click its Settings. Local history is always on. Set up Last.fm or ListenBrainz separately and enable the provider you want. Closing Activity does not disable a provider. Retry failed deliveries with the Retry failed control.","history scrobble scrobbling account credentials login authentication lastfm listenbrainz retry submissions"),
        ("gravity","Create a Gravity Queue","Playlist","Click Gravity","Build a related listening sequence from the queue.","Click Gravity in the playlist header and choose the available options. It reshapes the unplayed part of the queue while keeping the played prefix and membership. Save the resulting order as a normal playlist if you want to keep it.","smart related sonic similar journey shuffle sort algorithm"),
        ("playlist-actions","Open track actions","Playlist","Right-click track","Access context-specific queue and library actions.","Right-click a playlist track to open its action menu. Select an available action for that track. Tagged rows provide operations on multiple queue entries. Removing a queue row does not delete your audio files.","context menu actions metadata tags queue selected"),
        ("graphical","Switch graphical and terminal presentation","Appearance","F9","Change presentation while retaining the same player.","In a graphical session, F9 switches native graphics and cell presentation. Start a native session with staramp graphical in a compatible terminal such as Kitty. A terminal without graphics support uses the cell interface.","native classic rack graphics cell tui mode terminal fallback"),
        ("refresh","Refresh missing graphics","Appearance","Ctrl+L","Repaint controls and artwork without restarting playback.","Press Ctrl+L if your terminal loses image placements. KIT also repairs quiet regions automatically and repaints on terminal focus return. Playback ownership remains unchanged.","redraw repair corrupt missing visuals cover image display"),
        ("fold","Collapse or expand a graphical panel","Workspace","Panel fold control","Make room in the native rack without closing a panel.","Use the fold control in the graphical panel header. A folded panel keeps its playback and service state. Focus the panel and press Enter to expand it. Alt+P and the other panel toggles show or hide whole panels instead.","collapse expand rack minimize playlist window"),
        ("remote","Play a remote library over SSH","Library","","Connect from your shell to a remote library host.","Use staramp --ssh HOST to connect. The frontend and host must support compatible audio and graphics capabilities. Use staramp --help in your shell for CLI options and library management commands.","ssh remote server network music host connection"),
        ("scan","Index a music folder","Library","","Make local music available to the library browser.","In your shell run staramp scan ~/Music, using your music folder. Then launch staramp and press L (lowercase) to browse the indexed library. Scanning updates the rebuildable index without replacing listening history.","rescan scan import add directory folder files index collection"),
        ("keys","Read shortcut notation","Workspace","","Understand case-sensitive and panel-specific keys.","Lowercase letters are unmodified keys; uppercase letters mean Shift. For example L loads the whole library while l opens its browser. A key in the Equalizer, Album or Library can override the same key in Player. Focus shortcuts are Alt+1…5. Help consumes all typing until closed.","keyboard modifiers shift uppercase lowercase context shortcuts"),
    ] {
        commands.push(Command::new(id,title,group).shortcut(key).summary(summary).instructions(instructions).keywords(aliases));
    }
    Guide::new("STAR/AMP", commands)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn help_catalogue_covers_all_bindings_and_finds_functions() {
        let mut g = guide();
        assert!(g.results().len() >= keymap::BINDINGS.len() + keymap::MOUSE.len());
        for query in [
            "history",
            "scrobbling",
            "gravity",
            "profile",
            "visualizer",
            "cover",
            "ssh",
            "alt+s",
            "delete",
        ] {
            g.search(query);
            assert!(!g.results().is_empty(), "{query}");
        }
        assert_eq!(
            g.handle_named("char:q", 0),
            starkit::help::Outcome::Consumed
        );
        g.search("microphone");
        assert!(g.results().is_empty());
        g.search("");
        #[cfg(feature = "terminal-graphics")]
        if let Some(path) = std::env::var_os("STAR_HELP_PROOF_DIR") {
            use starkit::{
                native_surface::{style::Style, PixelRect},
                terminal_graphics::{
                    protocol::{Component, Viewport},
                    renderer::render_reference,
                },
            };
            std::fs::create_dir_all(&path).unwrap();
            let style = Style::default();
            for (name, query, width, height) in [
                ("amp-help", "", 1280, 800),
                ("amp-help-history", "history", 1280, 800),
                ("amp-help-small", "visualizer", 760, 500),
            ] {
                g.search(query);
                let v = Viewport {
                    width,
                    height,
                    columns: 128,
                    rows: 40,
                    ..Viewport::default()
                };
                let mut scene = style.scene(v);
                let r = Guide::native_rect(width as u16, height as u16);
                scene.components.push(Component::Menu {
                    rect: starkit::native_surface::style::logical_rect(
                        PixelRect::new(r.x, r.y, r.width, r.height),
                        v,
                    ),
                });
                style
                    .mount(&mut scene, g.native(&style, r.width, r.height), r.x, r.y)
                    .unwrap();
                render_reference(&scene)
                    .unwrap()
                    .save(std::path::PathBuf::from(&path).join(format!("{name}.png")))
                    .unwrap();
            }
        }
    }
}
