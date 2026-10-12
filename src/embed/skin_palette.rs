//! Shared Classic colors and button geometry, independent of the pixel renderer.
use super::{native, Palette};
use starkit::native_surface::{classic::Colors, PixelRect};

/// The original study keeps its artwork colors; theme mode uses the selected
/// palette directly so Classic's surface hues do not bleed into other themes.
pub(crate) const REFERENCE_PALETTE: Palette = Palette {
    bg: [23, 24, 32],
    inset_bg: None,
    fg: [215, 217, 207],
    muted: [152, 158, 172],
    accent: [164, 215, 145],
    selected: [48, 63, 61],
    border: [116, 121, 134],
    error: [224, 152, 133],
};
/// Named AMP treatment, recognized by its complete palette so embedded hosts
/// can use the same artwork without changing the surface protocol.
pub(crate) const SILVER_PALETTE: Palette = Palette {
    bg: [40, 40, 59],
    inset_bg: Some([6, 7, 11]),
    fg: [207, 216, 225],
    muted: [154, 161, 183],
    accent: [164, 215, 145],
    selected: [49, 49, 73],
    border: [110, 113, 137],
    error: [239, 73, 50],
};
pub(crate) fn tone(p: &Palette, original: &str, ink: bool) -> String {
    if *p != REFERENCE_PALETTE {
        return native::hex(if ink { p.fg } else { p.bg });
    }
    calibrated(
        original,
        &native::hex(if ink {
            REFERENCE_PALETTE.fg
        } else {
            REFERENCE_PALETTE.bg
        }),
        &native::hex(if ink { p.fg } else { p.bg }),
    )
}
pub(crate) fn generic(p: &Palette) -> Colors {
    Colors::new(p.bg, p.fg, p.muted, p.accent, p.border)
}
pub(crate) fn calibrated(original: &str, reference: &str, actual: &str) -> String {
    use starkit::theme::color::Rgb;
    let a = Rgb::parse_hex(original).expect("skin source color");
    let b = Rgb::parse_hex(reference).expect("reference theme color");
    let c = Rgb::parse_hex(actual).expect("resolved theme color");
    let channel = |a, b, c| (i16::from(a) + i16::from(c) - i16::from(b)).clamp(0, 255) as u8;
    Rgb::new(
        channel(a.r, b.r, c.r),
        channel(a.g, b.g, c.g),
        channel(a.b, b.b, c.b),
    )
    .to_hex()
}
pub(crate) fn colors(p: &Palette) -> Colors {
    if *p == SILVER_PALETTE {
        return Colors {
            panel: "#28283b".into(),
            inset: "#06070b".into(),
            ink: native::hex(p.fg),
            dim: native::hex(p.muted),
            accent: native::hex(p.accent),
            highlight: "#7e8299".into(),
            shadow: "#10101b".into(),
            raised: "#b7c6d0".into(),
            title: "#222235".into(),
        };
    }
    if *p != REFERENCE_PALETTE {
        use starkit::theme::color::Rgb;
        let bg = Rgb::new(p.bg[0], p.bg[1], p.bg[2]);
        let fg = Rgb::new(p.fg[0], p.fg[1], p.fg[2]);
        return Colors {
            panel: bg.to_hex(),
            inset: p
                .inset_bg
                .map(native::hex)
                .unwrap_or_else(|| bg.mix(Rgb::new(0, 0, 0), 0.06).to_hex()),
            ink: fg.to_hex(),
            dim: native::hex(p.muted),
            accent: native::hex(p.accent),
            highlight: bg.mix(fg, 0.16).to_hex(),
            shadow: bg.mix(Rgb::new(0, 0, 0), 0.25).to_hex(),
            raised: bg.mix(fg, 0.09).to_hex(),
            title: bg.mix(fg, 0.04).to_hex(),
        };
    }
    let a = generic(p);
    let r = generic(&REFERENCE_PALETTE);
    Colors {
        panel: calibrated("#303340", &r.panel, &a.panel),
        inset: if *p == REFERENCE_PALETTE {
            "#101813".into()
        } else {
            a.inset.clone()
        },
        ink: native::hex(p.fg),
        dim: native::hex(p.muted),
        accent: native::hex(p.accent),
        highlight: calibrated("#7c8294", &r.highlight, &a.highlight),
        shadow: calibrated("#101117", &r.shadow, &a.shadow),
        raised: calibrated("#474d5d", &r.raised, &a.raised),
        title: calibrated("#242733", &r.title, &a.title),
    }
}
pub(crate) fn inactive_led(p: &Palette) -> String {
    if *p == SILVER_PALETTE {
        return "#1c2a20".into();
    }
    if *p == REFERENCE_PALETTE {
        return "#1c2a20".into();
    }
    // Unlit segments follow the background hue, not Classic's green phosphor.
    use starkit::theme::color::Rgb;
    Rgb::parse_hex(&colors(p).inset)
        .unwrap()
        .mix(Rgb::new(p.fg[0], p.fg[1], p.fg[2]), 0.06)
        .to_hex()
}

/// Compact artwork inside the existing pointer target. Small fold controls
/// keep their size; ordinary buttons use the selected 22px Minimal treatment.
pub(crate) fn button_rect(rect: PixelRect) -> PixelRect {
    let height = rect.height.min(22);
    let width = if rect.width == rect.height {
        rect.width.min(22)
    } else {
        (rect.width * 4 / 5).max(height).min(rect.width)
    };
    PixelRect::new(
        rect.x + (rect.width - width) / 2,
        rect.y + (rect.height - height) / 2,
        width,
        height,
    )
}
