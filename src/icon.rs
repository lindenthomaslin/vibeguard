//! 品牌图：解码、调色，并给出两种显示方式。
//!
//! - 原生图片：`logo_image` / `avatar_image` 交给 `graphics` 模块，由终端的
//!   图形协议（iTerm2 / Kitty / Sixel）直接画原图。
//! - 字符退路：`avatar_lines` 用「四分之一方块」——一个字符格表示 2×2 像素，
//!   每格在 7 种二分法里挑误差最小的一种当前景/背景。同样的列数下横向细节
//!   是半块字符（`▀`）的两倍。
//!
//! 启动画面只在有图形协议时才放图：这张原图是一张细节很多的插画，缩到
//! 几十列宽的字符格就只剩一团粉色噪点，不如不放（见 `logo_size`）。
//!
//! 解码 + 调色在进程内只做一次（`OnceLock`）：这张图解码一次要几十毫秒，
//! 每帧重做一遍会让输入明显卡顿。

use image::imageops::FilterType;
use image::{DynamicImage, ImageReader, RgbaImage, imageops};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Mutex, OnceLock};

/// 标题栏头像的字符尺寸（像素是它的两倍宽、两倍高）
pub const AVATAR_COLS: u16 = 16;
pub const AVATAR_ROWS: u16 = 8;

/// 启动画面的宽度（列）。原生图片本身够清楚，不用铺太宽；字符格要多给——
/// 这张插画细节多，40 列以下就糊成一团粉了（实测 32 列完全认不出，
/// 48 列能看出侧脸和字标，56 列开始舒服）。
const LOGO_COLS_GRAPHICS: u16 = 32;
const LOGO_MIN_COLS_GRAPHICS: u16 = 16;
const LOGO_COLS_CELLS: u16 = 56;
const LOGO_MIN_COLS_CELLS: u16 = 40;

/// 字符格版本的工作分辨率：最大的单元格也就 56×28（112×56 像素），
/// 在 256² 上处理足够，也便宜。
const ART_SIDE: u32 = 256;

/// 所有处理的公共底图尺寸（最大输出约 320 px，1.5 倍够用）
const BASE_SIDE: u32 = 480;

const ICON_JPG: &[u8] = include_bytes!("../icon.jpg");

/// 头像的裁剪框：原图里脸所在的方块，按短边比例表示
const FACE_X: f32 = 0.494;
const FACE_Y: f32 = 0.335;
const FACE_SIZE: f32 = 0.431;

/// 四分之一方块：下标是四位掩码，第 0..3 位依次是左上、右上、左下、右下
const QUADRANTS: [char; 16] = [
    ' ', '▘', '▝', '▀', '▖', '▌', '▞', '▛', '▗', '▚', '▐', '▜', '▄', '▙', '▟', '█',
];

fn decoded() -> Option<&'static RgbaImage> {
    static SRC: OnceLock<Option<RgbaImage>> = OnceLock::new();
    SRC.get_or_init(|| {
        let reader = ImageReader::new(Cursor::new(ICON_JPG));
        Some(reader.with_guessed_format().ok()?.decode().ok()?.to_rgba8())
    })
    .as_ref()
}

/// 所有图片处理的公共底图：先缩到 640²。最大的一档输出也就 320 px，
/// 拿 1254² 的原图去模糊/排序纯属浪费（debug 下能多花两秒）。
fn base() -> Option<&'static RgbaImage> {
    static BASE: OnceLock<Option<RgbaImage>> = OnceLock::new();
    BASE.get_or_init(|| {
        let src = decoded()?;
        // Triangle 而不是 Lanczos：2 倍降采样看不出差别，debug 下快一个数量级
        Some(imageops::resize(
            src,
            BASE_SIDE,
            BASE_SIDE,
            FilterType::Triangle,
        ))
    })
    .as_ref()
}

/// 标题栏头像用的方图：裁到脸上，调色后对比更硬
fn face() -> Option<&'static RgbaImage> {
    static FACE: OnceLock<Option<RgbaImage>> = OnceLock::new();
    FACE.get_or_init(|| {
        let img = base()?;
        let side = (img.width().min(img.height()) as f32 * FACE_SIZE) as u32;
        let x = ((img.width() as f32 * FACE_X) as u32).min(img.width().saturating_sub(side));
        let y = ((img.height() as f32 * FACE_Y) as u32).min(img.height().saturating_sub(side));
        let mut face = imageops::crop_imm(img, x, y, side, side).to_image();
        grade(&mut face, 0.14, 0.97, 1.2, 1.0);
        Some(face)
    })
    .as_ref()
}

/// 启动画面用的整张图：自带手写的 vibeguard，本身就是完整的品牌块。
/// 和字符格版本用同一套调子（反锐化 + 压亮度 + 提饱和），只是半径按分辨率放大、
/// 强度收一点——大图上按字符格的力度锐化会出一圈光晕。
fn logo() -> Option<&'static RgbaImage> {
    static LOGO: OnceLock<Option<RgbaImage>> = OnceLock::new();
    LOGO.get_or_init(|| {
        let mut img = base()?.clone();
        let sigma = img.width() as f32 / 32.0;
        sharpen(&mut img, sigma, 1.1);
        grade(&mut img, 0.30, 1.0, 1.5, 1.5);
        Some(img)
    })
    .as_ref()
}

/// 字符格版本用的图。原图的局部对比度被压得很扁（亮度 p5=178、p50=218，
/// 只有 0.03% 的像素低于 80），只做全局拉伸的话缩到字符格还是一团粉。
/// 先在 256² 上做一次大半径反锐化，把发丝、眼睛、字标这些局部结构拉出来，
/// 再提饱和——这一步是「40 列能看出是个人」和「一团噪点」的分界线。
fn logo_art() -> Option<&'static RgbaImage> {
    static ART: OnceLock<Option<RgbaImage>> = OnceLock::new();
    ART.get_or_init(|| {
        let mut img = imageops::resize(base()?, ART_SIDE, ART_SIDE, FilterType::Lanczos3);
        // 先反锐化把局部结构拉出来，再把亮度压到深色终端能接受的区间：
        // 原图 p50 亮度 218/255，直接贴上去就是一块发光的粉板。
        sharpen(&mut img, 8.0, 1.8);
        grade(&mut img, 0.30, 1.0, 1.7, 1.7);
        Some(img)
    })
    .as_ref()
}

/// 大半径低通：缩到很小再放大回去。反锐化只需要一个模糊版本，用降采样代替
/// 高斯能省两个数量级的像素运算（debug 下尤其明显）。
/// 缩略后每个格子覆盖 `sigma * 3` 个像素——一个边长 d 的方框，标准差约 d/3.5，
/// 这样才和「高斯 sigma」大致对得上。
fn lowpass(img: &RgbaImage, sigma: f32) -> RgbaImage {
    let (w, h) = (img.width(), img.height());
    let step = (sigma * 3.0).max(1.0);
    let small = (
        ((w as f32 / step).round() as u32).max(1),
        ((h as f32 / step).round() as u32).max(1),
    );
    let tiny = imageops::resize(img, small.0, small.1, FilterType::Triangle);
    imageops::resize(&tiny, w, h, FilterType::Triangle)
}

/// 大半径反锐化：`img + (img - blur) * amount`，只动 RGB，不动 alpha。
fn sharpen(img: &mut RgbaImage, sigma: f32, amount: f32) {
    let blurred = lowpass(img, sigma);
    for (p, b) in img.pixels_mut().zip(blurred.pixels()) {
        for k in 0..3 {
            let v = p.0[k] as f32 + (p.0[k] as f32 - b.0[k] as f32) * amount;
            p.0[k] = v.clamp(0.0, 255.0) as u8;
        }
    }
}

/// 调好色的方图，交给终端的图形协议直接显示
pub fn logo_image() -> Option<&'static DynamicImage> {
    static IMAGE: OnceLock<Option<DynamicImage>> = OnceLock::new();
    IMAGE
        .get_or_init(|| logo().map(|f| DynamicImage::ImageRgba8(f.clone())))
        .as_ref()
}

/// 同 `logo_image`，但裁成圆形，用在标题栏的小头像上
pub fn avatar_image() -> Option<&'static DynamicImage> {
    static AVATAR: OnceLock<Option<DynamicImage>> = OnceLock::new();
    AVATAR
        .get_or_init(|| {
            let mut img = face()?.clone();
            round_corners(&mut img);
            Some(DynamicImage::ImageRgba8(img))
        })
        .as_ref()
}

/// 启动画面的尺寸（列, 行）。图是方的，字符格约 1:2，所以行数取列数的一半。
/// 终端太矮时返回 `None`：一张被切掉半张脸的图不如不放。
pub fn logo_size(term_height: u16, native: bool) -> Option<(u16, u16)> {
    let (preferred, min) = if native {
        (LOGO_COLS_GRAPHICS, LOGO_MIN_COLS_GRAPHICS)
    } else {
        (LOGO_COLS_CELLS, LOGO_MIN_COLS_CELLS)
    };
    // 标题栏 10 行、输入框 3 行、状态栏 1 行，图下面一行说明，
    // 再留两行余量——贴着输入框框线太挤
    let room = term_height.saturating_sub(17);
    let mut cols = preferred;
    while cols > min && cols / 2 + 1 > room {
        cols -= 1;
    }
    (cols / 2 + 1 <= room).then_some((cols, cols / 2))
}

/// 启动画面的字符画版本，每个尺寸只渲染一次
pub fn logo_lines(size: (u16, u16)) -> &'static [Line<'static>] {
    static CACHE: OnceLock<Mutex<HashMap<(u16, u16), &'static [Line<'static>]>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(lines) = cache.get(&size) {
        return lines;
    }
    let lines = logo_art()
        .map(|img| render_cells(img, size.0 as usize, size.1 as usize))
        .unwrap_or_default();
    // 尺寸只有寥寥几种，泄漏一份换取 `&'static`
    let leaked: &'static [Line<'static>] = Box::leak(lines.into_boxed_slice());
    cache.insert(size, leaked);
    leaked
}

/// 标题栏的小头像，第一次调用时渲染
pub fn avatar_lines() -> &'static [Line<'static>] {
    static ICON: OnceLock<&'static [Line<'static>]> = OnceLock::new();
    ICON.get_or_init(|| {
        let lines = face().map_or_else(Vec::new, |img| {
            let mut img = img.clone();
            round_corners(&mut img);
            render_cells(&img, AVATAR_COLS as usize, AVATAR_ROWS as usize)
        });
        Box::leak(lines.into_boxed_slice())
    })
}

/// 把图缩到 2·cols × 2·rows 像素，每格 4 个像素合成一个四分之一方块
fn render_cells(img: &RgbaImage, cols: usize, rows: usize) -> Vec<Line<'static>> {
    if cols == 0 || rows == 0 {
        return Vec::new();
    }
    let small = imageops::resize(
        img,
        (cols * 2) as u32,
        (rows * 2) as u32,
        imageops::FilterType::Lanczos3,
    );
    let mut out = Vec::with_capacity(rows);
    for row in 0..rows {
        let mut spans = Vec::with_capacity(cols);
        for col in 0..cols {
            let (x, y) = ((col * 2) as u32, (row * 2) as u32);
            spans.push(quadrant([
                *small.get_pixel(x, y),
                *small.get_pixel(x + 1, y),
                *small.get_pixel(x, y + 1),
                *small.get_pixel(x + 1, y + 1),
            ]));
        }
        out.push(Line::from(spans));
    }
    out
}

/// 四个像素合成一个字符：选一种二分法，让两种颜色的误差最小
fn quadrant(q: [image::Rgba<u8>; 4]) -> Span<'static> {
    let opaque = q
        .iter()
        .enumerate()
        .filter(|(_, p)| p.0[3] >= 128)
        .fold(0u8, |m, (i, _)| m | 1 << i);
    if opaque == 0 {
        return Span::raw(" ");
    }
    let rgb = q.map(|p| [p.0[0] as f32, p.0[1] as f32, p.0[2] as f32]);

    // 有透明的格子：只画不透明那部分，颜色取平均
    if opaque != 0b1111 {
        let fg = mean(&rgb, opaque);
        return Span::styled(
            QUADRANTS[opaque as usize].to_string(),
            Style::new().fg(rgb_color(fg)),
        );
    }

    // 四格都不透明：在 7 种二分法里挑误差最小的（掩码取反等于换前后景，不必重算）
    let mut best = (f32::MAX, 0u8, [0.0; 3], [0.0; 3]);
    for mask in 1..8u8 {
        let fg = mean(&rgb, mask);
        let bg = mean(&rgb, !mask & 0b1111);
        let err: f32 = (0..4)
            .map(|i| {
                let m = if mask >> i & 1 == 1 { fg } else { bg };
                (0..3).map(|k| (rgb[i][k] - m[k]).powi(2)).sum::<f32>()
            })
            .sum();
        if err < best.0 {
            best = (err, mask, fg, bg);
        }
    }
    let (_, mask, fg, bg) = best;
    Span::styled(
        QUADRANTS[mask as usize].to_string(),
        Style::new().fg(rgb_color(fg)).bg(rgb_color(bg)),
    )
}

fn mean(rgb: &[[f32; 3]; 4], mask: u8) -> [f32; 3] {
    let mut sum = [0.0; 3];
    let mut n = 0.0;
    for (i, px) in rgb.iter().enumerate() {
        if mask >> i & 1 == 1 {
            for k in 0..3 {
                sum[k] += px[k];
            }
            n += 1.0;
        }
    }
    if n == 0.0 {
        return [0.0; 3];
    }
    [sum[0] / n, sum[1] / n, sum[2] / n]
}

fn rgb_color([r, g, b]: [f32; 3]) -> Color {
    let c = |v: f32| v.clamp(0.0, 255.0) as u8;
    Color::Rgb(c(r), c(g), c(b))
}

/// 2% / 98% 亮度分位。用 256 格直方图，别排序——几十万像素排序在 debug 下很贵。
fn percentiles(img: &RgbaImage) -> (f32, f32) {
    let mut hist = [0u32; 256];
    for p in img.pixels() {
        let bin = (luma(p.0) * 255.0).round().clamp(0.0, 255.0) as usize;
        hist[bin] += 1;
    }
    let total: u32 = hist.iter().sum();
    let at = |frac: f32| -> f32 {
        let want = (total as f32 * frac) as u32;
        let mut seen = 0;
        for (v, n) in hist.iter().enumerate() {
            seen += n;
            if seen > want {
                return v as f32 / 255.0;
            }
        }
        1.0
    };
    (at(0.02), at(0.98))
}

/// 原图是很淡的粉白：按亮度分位把它拉到终端能分辨的区间，再提饱和度。
/// 只拉伸亮度、按比例缩放 RGB，色相不会被拉偏（逐通道拉伸会把粉色拉成青色）。
fn grade(img: &mut RgbaImage, floor: f32, ceil: f32, sat: f32, g: f32) {
    let (lo, hi) = percentiles(img);
    let span = (hi - lo).max(1e-3);
    // 伽马查表，省得每像素再算一次 powf
    let mut lut = [0u8; 256];
    for (v, out) in lut.iter_mut().enumerate() {
        *out = to_u8((v as f32 / 255.0).powf(g));
    }

    // 拉伸 → 提饱和 → 伽马，一遍扫完（分成三遍在 debug 下要多花两倍时间）
    for p in img.pixels_mut() {
        let [r, g, b, a] = p.0;
        let (r, gg, bb) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
        let l = 0.299 * r + 0.587 * gg + 0.114 * bb;
        let t = ((l - lo) / span).clamp(0.0, 1.0);
        let k = (floor + t * (ceil - floor)) / l.max(0.03);
        let (r, gg, bb) = (r * k, gg * k, bb * k);
        let l2 = 0.299 * r + 0.587 * gg + 0.114 * bb;
        let f = |c: f32| lut[to_u8(l2 + (c - l2) * sat) as usize];
        *p = image::Rgba([f(r), f(gg), f(bb), a]);
    }
}

fn to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0) as u8
}

fn luma([r, g, b, _]: [u8; 4]) -> f32 {
    (0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32) / 255.0
}

/// 圆形裁切：头像四周的浅色背景不会在深色终端里糊成一个方块
fn round_corners(img: &mut RgbaImage) {
    let (w, h) = (img.width() as f32, img.height() as f32);
    let radius = w.min(h) * 0.5;
    for y in 0..img.height() {
        for x in 0..img.width() {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let dx = px - px.clamp(radius, w - radius);
            let dy = py - py.clamp(radius, h - radius);
            if (dx * dx + dy * dy).sqrt() > radius {
                img.get_pixel_mut(x, y).0[3] = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;

    fn spans(ls: &[Line<'static>], w: usize, h: usize) {
        assert_eq!(ls.len(), h);
        for l in ls {
            assert_eq!(l.spans.len(), w);
        }
    }

    #[test]
    fn header_icon_matches_declared_size() {
        spans(avatar_lines(), AVATAR_COLS as usize, AVATAR_ROWS as usize);
    }

    /// 选出来的那一档必须能塞进终端剩余的行数里，太矮就干脆不显示
    #[test]
    fn logo_fits_the_available_rows() {
        for term in [20u16, 24, 30, 34, 36, 40, 46, 60] {
            for native in [false, true] {
                match logo_size(term, native) {
                    Some((cols, rows)) => {
                        assert_eq!(rows, cols / 2, "方图的行数应当是列数的一半");
                        assert!(rows + 1 <= term - 17, "终端 {term} 行放不下 {rows} 行图");
                        let min = if native {
                            LOGO_MIN_COLS_GRAPHICS
                        } else {
                            LOGO_MIN_COLS_CELLS
                        };
                        assert!(cols >= min, "字符格太窄就认不出了");
                    }
                    None => assert!(term < 40, "终端 {term} 行应当放得下"),
                }
            }
        }
    }

    /// 字符格版本要多给几十列：40 列以下这张插画认不出来，所以矮终端直接不放
    #[test]
    fn the_cell_version_needs_more_columns_than_the_native_one() {
        assert!(logo_size(30, false).is_none(), "30 行放不下 40 列的图");
        assert!(logo_size(30, true).is_some(), "原生图片 30 行够了");
        let (cells, _) = logo_size(60, false).unwrap();
        let (native, _) = logo_size(60, true).unwrap();
        assert!(cells > native, "字符格要比原生图片宽：{cells} vs {native}");
        assert!(cells >= 40);
    }

    #[test]
    fn logo_is_skipped_on_short_terminals() {
        assert!(logo_size(24, false).is_none(), "24 行放不下，应当不显示");
        assert!(logo_size(24, true).is_none(), "24 行放不下，应当不显示");
        assert!(logo_size(40, false).is_some());
    }

    /// 图片必须只解码一次：每帧解码一张大图会让输入卡顿
    #[test]
    fn image_is_decoded_only_once() {
        assert!(std::ptr::eq(
            avatar_lines().as_ptr(),
            avatar_lines().as_ptr()
        ));
        assert!(std::ptr::eq(logo_image().unwrap(), logo_image().unwrap()));
        assert!(std::ptr::eq(
            avatar_image().unwrap(),
            avatar_image().unwrap()
        ));
    }

    /// 原生图片走的是同一张方图；头像版本是其中的一块方裁切，多一道圆形裁边
    #[test]
    fn native_images_are_square_and_the_avatar_is_round() {
        let logo = logo_image().unwrap();
        assert_eq!(logo.width(), logo.height());
        let avatar = avatar_image().unwrap();
        assert_eq!(avatar.width(), avatar.height(), "头像也必须是方的");
        assert!(avatar.width() < logo.width(), "头像只是原图里脸那一块");
        let (w, h) = (avatar.width(), avatar.height());
        assert_eq!(avatar.get_pixel(0, 0).0[3], 0, "四角应当是透明的");
        assert!(avatar.get_pixel(w / 2, h / 2).0[3] > 0, "中间应当有内容");
    }

    /// 字符画只能用四分之一方块那 16 个字形，且每格一个
    #[test]
    fn cells_use_quadrant_glyphs_only() {
        let ls = avatar_lines();
        spans(ls, AVATAR_COLS as usize, AVATAR_ROWS as usize);
        for l in ls {
            for s in &l.spans {
                let ch = s.content.chars().next().unwrap();
                assert!(QUADRANTS.contains(&ch), "意外的字形 {ch:?}");
            }
        }
    }

    /// 四格四色时挑出来的二分法必须比随便一种更接近原图
    #[test]
    fn quadrant_picks_the_closest_two_colour_split() {
        // 上半黑、下半白：水平劈开（掩码 0b0011）是精确解
        let q = [
            image::Rgba([0, 0, 0, 255]),
            image::Rgba([0, 0, 0, 255]),
            image::Rgba([255, 255, 255, 255]),
            image::Rgba([255, 255, 255, 255]),
        ];
        let span = quadrant(q);
        assert_eq!(span.content.as_ref(), "▀");
        assert_eq!(span.style.fg, Some(Color::Rgb(0, 0, 0)));
        assert_eq!(span.style.bg, Some(Color::Rgb(255, 255, 255)));
    }

    /// 全透明的格子留空，不能糊成一块黑
    #[test]
    fn transparent_cells_stay_empty() {
        let clear = image::Rgba([0, 0, 0, 0]);
        let span = quadrant([clear; 4]);
        assert_eq!(span.content.as_ref(), " ");
        assert_eq!(span.style.fg, None);
    }
}
