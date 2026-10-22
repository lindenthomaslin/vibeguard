//! 全屏布局：顶部图标栏、中间对话区、选择区、固定在底部的输入框和状态栏。
//! 只做矩形计算，不碰终端，便于测试。

use crate::icon;
use ratatui::layout::Rect;

/// 终端高度不足时，图标栏收成一行
pub const FULL_HEADER_MIN_HEIGHT: u16 = 20;
/// 对话区至少保留的行数（空间够时）
const MIN_BODY: u16 = 3;

#[derive(Debug, PartialEq, Eq)]
pub struct Areas {
    pub header: Rect,
    pub body: Rect,
    /// 选择区（含边框），没有选择时高度为 0
    pub choice: Rect,
    /// 输入框（含边框）
    pub input: Rect,
    pub status: Rect,
}

/// `input_rows` 是输入文字折行后的行数，`choice_rows` 是选择区需要的总高度（含边框）。
/// `compact_header` 为 true 时图标栏强制收成一行，把空间让给对话区。
pub fn areas(area: Rect, input_rows: u16, choice_rows: u16, compact_header: bool) -> Areas {
    let mut rest = area.height;
    let mut take = |want: u16| {
        let h = want.min(rest);
        rest -= h;
        h
    };

    let status_h = take(1);
    let max_input = (area.height / 3).max(3);
    let input_h = take(input_rows.saturating_add(2).clamp(3, max_input));

    // 对话区优先于图标栏和选择区：它们只使用剩余空间里超出 MIN_BODY 的部分
    let header_want =
        if !compact_header && area.height >= FULL_HEADER_MIN_HEIGHT && area.width >= 50 {
            icon::AVATAR_ROWS + 2
        } else {
            1
        };
    let header_h = header_want.min(rest.saturating_sub(MIN_BODY));
    rest -= header_h;
    let choice_h = choice_rows
        .min(area.height / 2)
        .min(rest.saturating_sub(MIN_BODY));
    rest -= choice_h;
    let body_h = rest;

    let rect = |y: u16, h: u16| Rect::new(area.x, area.y + y, area.width, h);
    let header = rect(0, header_h);
    let body = rect(header_h, body_h);
    let choice = rect(header_h + body_h, choice_h);
    let input = rect(header_h + body_h + choice_h, input_h);
    let status = rect(header_h + body_h + choice_h + input_h, status_h);
    Areas {
        header,
        body,
        choice,
        input,
        status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn total_height(a: &Areas) -> u16 {
        a.header.height + a.body.height + a.choice.height + a.input.height + a.status.height
    }

    fn contiguous(a: &Areas) -> bool {
        a.body.y == a.header.y + a.header.height
            && a.choice.y == a.body.y + a.body.height
            && a.input.y == a.choice.y + a.choice.height
            && a.status.y == a.input.y + a.input.height
    }

    #[test]
    fn large_terminal_uses_full_header() {
        let a = areas(Rect::new(0, 0, 100, 40), 1, 0, false);
        assert_eq!(a.header.height, icon::AVATAR_ROWS + 2);
        assert_eq!(a.input.height, 3);
        assert_eq!(a.status.height, 1);
        assert_eq!(a.choice.height, 0);
        assert_eq!(total_height(&a), 40);
        assert!(contiguous(&a));
    }

    #[test]
    fn input_stays_pinned_to_the_bottom_whatever_is_above() {
        for choice in [0, 6] {
            for compact in [false, true] {
                let a = areas(Rect::new(0, 0, 100, 40), 1, choice, compact);
                assert_eq!(a.status.y, 39);
                assert_eq!(a.input.y + a.input.height, 39);
            }
        }
    }

    #[test]
    fn short_terminal_collapses_header_to_one_line() {
        let a = areas(Rect::new(0, 0, 100, 12), 1, 0, false);
        assert_eq!(a.header.height, 1);
        assert!(a.body.height >= MIN_BODY);
        assert_eq!(total_height(&a), 12);
    }

    #[test]
    fn narrow_terminal_collapses_header_too() {
        let a = areas(Rect::new(0, 0, 40, 40), 1, 0, false);
        assert_eq!(a.header.height, 1);
    }

    #[test]
    fn compact_flag_forces_one_line_header_and_gives_the_space_to_the_body() {
        let full = areas(Rect::new(0, 0, 100, 40), 1, 6, false);
        let compact = areas(Rect::new(0, 0, 100, 40), 1, 6, true);
        assert_eq!(compact.header.height, 1);
        assert_eq!(
            compact.body.height,
            full.body.height + full.header.height - 1
        );
        assert_eq!(total_height(&compact), 40);
        assert!(contiguous(&compact));
    }

    #[test]
    fn input_grows_with_text_but_is_capped_to_a_third() {
        let small = areas(Rect::new(0, 0, 100, 40), 2, 0, false);
        assert_eq!(small.input.height, 4);
        let big = areas(Rect::new(0, 0, 100, 40), 30, 0, false);
        assert_eq!(big.input.height, 13);
        assert_eq!(total_height(&big), 40);
    }

    #[test]
    fn choice_area_takes_space_from_body_but_keeps_minimum() {
        let a = areas(Rect::new(0, 0, 100, 40), 1, 6, false);
        assert_eq!(a.choice.height, 6);
        assert_eq!(total_height(&a), 40);

        let tight = areas(Rect::new(0, 0, 100, 10), 1, 6, false);
        assert!(tight.body.height >= MIN_BODY);
        assert_eq!(total_height(&tight), 10);
    }

    #[test]
    fn tiny_terminals_never_overflow_or_panic() {
        for h in 0..8 {
            for w in [1, 20, 80] {
                for compact in [false, true] {
                    let a = areas(Rect::new(0, 0, w, h), 5, 6, compact);
                    assert!(total_height(&a) <= h, "{w}x{h}");
                    assert!(contiguous(&a), "{w}x{h}");
                }
            }
        }
    }

    #[test]
    fn respects_area_offset() {
        let a = areas(Rect::new(3, 2, 50, 30), 1, 0, false);
        assert_eq!(a.header.x, 3);
        assert_eq!(a.header.y, 2);
        assert_eq!(a.status.y + a.status.height, 32);
    }
}
