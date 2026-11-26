use super::*;

struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    original: HGDIOBJ,
    size: (i32, i32),
    last: Option<(usize, u32)>,
}

impl Surface {
    unsafe fn new(target: HDC, size: (i32, i32)) -> Option<Self> {
        let dc = CreateCompatibleDC(target);
        if dc.is_null() {
            return None;
        }
        let bitmap = CreateCompatibleBitmap(target, size.0, size.1);
        let original = if bitmap.is_null() {
            null_mut()
        } else {
            SelectObject(dc, bitmap)
        };
        if original.is_null() || original as isize == -1 {
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            DeleteDC(dc);
            return None;
        }
        Some(Self {
            dc,
            bitmap,
            original,
            size,
            last: None,
        })
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.original);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

thread_local! { static SURFACE: RefCell<Option<Surface>> = const { RefCell::new(None) }; }
pub fn clear() {
    SURFACE.with_borrow_mut(|surface| {
        surface.take();
    });
}

pub unsafe fn paint(dc: HDC, rect: RECT, id: usize, frame: u32) {
    let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
    let Some(size) = geometry::render_size(w, h) else {
        return;
    };
    let time = frame as f32 * STOCK_INTERVAL_MS as f32 / 1000.;
    SURFACE.with_borrow_mut(|slot| {
        if slot.as_ref().is_none_or(|s| s.size != size) {
            slot.take();
            *slot = Surface::new(dc, size);
        }
        let Some(surface) = slot.as_mut() else {
            draw(dc, rect, id, time);
            return;
        };
        if surface.last != Some((id, frame)) {
            draw(
                surface.dc,
                RECT {
                    left: 0,
                    top: 0,
                    right: size.0,
                    bottom: size.1,
                },
                id,
                time,
            );
            surface.last = Some((id, frame));
        }
        SetStretchBltMode(dc, COLORONCOLOR);
        StretchBlt(
            dc, rect.left, rect.top, w, h, surface.dc, 0, 0, size.0, size.1, SRCCOPY,
        );
    });
}

pub unsafe fn draw(mem: HDC, rect: RECT, id: usize, t: f32) {
    let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
    if width <= 0 || height <= 0 {
        return;
    }
    let saved = SaveDC(mem);
    IntersectClipRect(mem, rect.left, rect.top, rect.right, rect.bottom);
    SetViewportOrgEx(mem, rect.left, rect.top, null_mut());
    let colors = [(51., 212., 164.), (246., 141., 77.), (62., 157., 212.)];
    let (r, g, b) = colors[id];
    SetDCPenColor(mem, 0);
    SelectObject(mem, GetStockObject(NULL_PEN));
    SelectObject(mem, GetStockObject(DC_BRUSH));
    SetDCBrushColor(mem, 0x211009);
    Rectangle(mem, -1, -1, width + 1, height + 1);
    for j in 0..72 {
        let x = ((j * 197 % 811) as f32 / 811. * width as f32) as i32;
        let y = ((j * 113 % 421) as f32 / 421. * height as f32 * 0.6) as i32;
        SetPixelV(mem, x, y, 0xc4ae99);
    }
    if id == 1 {
        SetDCBrushColor(mem, 0x72aeff);
        let x = width * 3 / 4;
        let y = height / 4;
        let s = height / 12;
        Ellipse(mem, x - s, y - s, x + s, y + s);
    }
    // Each layer shifts the same two waves. Compute their angles once per column.
    let columns: [_; 81] = std::array::from_fn(|j| {
        let x = j as f32 / 80.;
        (
            (width as f32 * x) as i32,
            (x * (4. + id as f32 * 2.) + t * 0.18).sin_cos(),
            (x * 9. - t * 0.13).sin_cos(),
        )
    });
    for layer in 0..26 {
        let k = layer as f32 / 25.;
        let f = 1. - 0.83 * k;
        SetDCBrushColor(
            mem,
            (r * f) as u32 | ((g * f) as u32) << 8 | ((b * f) as u32) << 16,
        );
        let mut points = [POINT { x: 0, y: 0 }; 83];
        let (sa, ca) = (k * 2.).sin_cos();
        let (sb, cb) = k.sin_cos();
        for (p, &(x, (s1, c1), (s2, c2))) in points[..81].iter_mut().zip(&columns) {
            p.x = x;
            p.y = (height as f32
                * (0.42 + k * 0.52 + 0.09 * (s1 * ca + c1 * sa) + 0.035 * (c2 * cb - s2 * sb)))
                as i32;
        }
        points[81] = POINT {
            x: width,
            y: height,
        };
        points[82] = POINT { x: 0, y: height };
        Polygon(mem, points.as_ptr(), 83);
    }
    RestoreDC(mem, saved);
}
