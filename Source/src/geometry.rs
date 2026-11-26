pub const DISPLAY_FPS: u32 = 12;
pub const LONG_EDGE: i32 = 1280;
pub const SHORT_EDGE: i32 = 720;

/// Fit either orientation inside the rendering budget, preserving the source aspect.
pub fn render_size(w: i32, h: i32) -> Option<(i32, i32)> {
    if w <= 0 || h <= 0 {
        return None;
    }
    let (mw, mh) = if w >= h {
        (LONG_EDGE, SHORT_EDGE)
    } else {
        (SHORT_EDGE, LONG_EDGE)
    };
    let scale = 1.0_f64.min(mw as f64 / w as f64).min(mh as f64 / h as f64);
    Some((
        ((w as f64 * scale).round() as i32).max(1),
        ((h as f64 * scale).round() as i32).max(1),
    ))
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Fit {
    #[default]
    Fill,
    Fit,
    Stretch,
}
impl Fit {
    pub fn name(self) -> &'static str {
        match self {
            Self::Fill => "Fill",
            Self::Fit => "Fit",
            Self::Stretch => "Stretch",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s {
            "Fit" => Self::Fit,
            "Stretch" => Self::Stretch,
            _ => Self::Fill,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Box2 {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}
impl Box2 {
    pub fn whole(w: i32, h: i32) -> Self {
        Self { x: 0, y: 0, w, h }
    }
}
pub fn placement(
    source: (i32, i32),
    display_aspect: f64,
    target: (i32, i32),
    fit: Fit,
) -> Option<(Box2, Box2)> {
    let (sw, sh) = source;
    let (tw, th) = target;
    if [sw, sh, tw, th].iter().any(|&v| v <= 0) {
        return None;
    }
    let mut src = Box2::whole(sw, sh);
    let mut dst = Box2::whole(tw, th);
    let aspect = if display_aspect.is_finite() && display_aspect > 0. {
        display_aspect
    } else {
        sw as f64 / sh as f64
    };
    let dest_aspect = tw as f64 / th as f64;
    let size = |n: f64, limit: i32| (n.round() as i32).clamp(1, limit);
    match fit {
        Fit::Fill if aspect > dest_aspect => {
            src.w = size(sw as f64 * dest_aspect / aspect, sw);
            src.x = (sw - src.w) / 2;
        }
        Fit::Fill => {
            src.h = size(sh as f64 * aspect / dest_aspect, sh);
            src.y = (sh - src.h) / 2;
        }
        Fit::Fit if aspect > dest_aspect => {
            dst.h = size(tw as f64 / aspect, th);
            dst.y = (th - dst.h) / 2;
        }
        Fit::Fit => {
            dst.w = size(th as f64 * aspect, tw);
            dst.x = (tw - dst.w) / 2;
        }
        Fit::Stretch => {}
    }
    Some((src, dst))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rendering_budget_handles_all_orientations_without_upscaling() {
        for (source, expected) in [
            ((3840, 2160), (1280, 720)),
            ((2160, 3840), (720, 1280)),
            ((8192, 8192), (720, 720)),
            ((5120, 1440), (1280, 360)),
            ((160, 90), (160, 90)),
            ((16384, 1), (1280, 1)),
        ] {
            assert_eq!(render_size(source.0, source.1), Some(expected));
        }
        assert!(render_size(0, 1080).is_none());
    }
    #[test]
    fn portrait_fill_crops_vertical() {
        let (s, d) = placement((1080, 1920), 0., (1600, 900), Fit::Fill).unwrap();
        assert_eq!((s.w, s.h, s.y), (1080, 608, 656));
        assert_eq!(d, Box2::whole(1600, 900));
    }
    #[test]
    fn fit_adds_bars_and_stretch_uses_both_bounds() {
        let (_, d) = placement((1280, 720), 0., (800, 600), Fit::Fit).unwrap();
        assert_eq!((d.h, d.y), (450, 75));
        assert_eq!(
            placement((1280, 720), 0., (800, 600), Fit::Stretch)
                .unwrap()
                .1,
            Box2::whole(800, 600)
        );
    }
    #[test]
    fn anamorphic_and_zero_size() {
        assert!(placement((0, 600), 0., (100, 100), Fit::Fill).is_none());
        assert_eq!(
            placement((720, 576), 16. / 9., (160, 90), Fit::Fit)
                .unwrap()
                .1,
            Box2::whole(160, 90)
        );
    }
}
