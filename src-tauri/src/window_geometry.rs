use serde::Serialize;

pub const WINDOW_MARGIN: i32 = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkArea {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl WorkArea {
    fn right(self) -> i64 {
        self.x as i64 + self.width as i64
    }

    fn bottom(self) -> i64 {
        self.y as i64 + self.height as i64
    }
}

fn window_right(position: Point, size: Size) -> i64 {
    position.x as i64 + size.width as i64
}

fn window_bottom(position: Point, size: Size) -> i64 {
    position.y as i64 + size.height as i64
}

fn intersection_area(position: Point, size: Size, area: WorkArea) -> i64 {
    let left = i64::from(position.x).max(i64::from(area.x));
    let top = i64::from(position.y).max(i64::from(area.y));
    let right = window_right(position, size).min(area.right());
    let bottom = window_bottom(position, size).min(area.bottom());
    (right - left).max(0) * (bottom - top).max(0)
}

fn distance_to_area_center(position: Point, size: Size, area: WorkArea) -> i128 {
    let window_center_x = i128::from(position.x) * 2 + i128::from(size.width);
    let window_center_y = i128::from(position.y) * 2 + i128::from(size.height);
    let area_center_x = i128::from(area.x) * 2 + i128::from(area.width);
    let area_center_y = i128::from(area.y) * 2 + i128::from(area.height);
    let dx = window_center_x - area_center_x;
    let dy = window_center_y - area_center_y;
    dx * dx + dy * dy
}

pub fn select_work_area(position: Point, size: Size, areas: &[WorkArea]) -> Option<WorkArea> {
    let visible = areas
        .iter()
        .copied()
        .map(|area| (intersection_area(position, size, area), area))
        .max_by_key(|(intersection, _)| *intersection);

    if let Some((intersection, area)) = visible {
        if intersection > 0 {
            return Some(area);
        }
    }

    areas
        .iter()
        .copied()
        .min_by_key(|area| distance_to_area_center(position, size, *area))
}

pub fn clamp_to_work_area(position: Point, size: Size, area: WorkArea) -> Point {
    let left = area.x.saturating_add(WINDOW_MARGIN);
    let top = area.y.saturating_add(WINDOW_MARGIN);
    let right = area
        .right()
        .saturating_sub(i64::from(size.width))
        .saturating_sub(i64::from(WINDOW_MARGIN));
    let bottom = area
        .bottom()
        .saturating_sub(i64::from(size.height))
        .saturating_sub(i64::from(WINDOW_MARGIN));

    Point {
        x: if right < i64::from(left) {
            area.x
        } else {
            i64::from(position.x).clamp(i64::from(left), right) as i32
        },
        y: if bottom < i64::from(top) {
            area.y
        } else {
            i64::from(position.y).clamp(i64::from(top), bottom) as i32
        },
    }
}

pub fn bottom_right(size: Size, area: WorkArea) -> Point {
    clamp_to_work_area(
        Point {
            x: area.right().saturating_sub(i64::from(size.width)) as i32,
            y: area.bottom().saturating_sub(i64::from(size.height)) as i32,
        },
        size,
        area,
    )
}

pub fn bottom_right_in_selected_work_area(
    restored_position: Point,
    size: Size,
    areas: &[WorkArea],
) -> Option<Point> {
    select_work_area(restored_position, size, areas).map(|area| bottom_right(size, area))
}

pub fn resize_preserving_nearest_edges(
    old_position: Point,
    old_size: Size,
    new_size: Size,
    area: WorkArea,
) -> Point {
    let distance_left = i64::from(old_position.x) - i64::from(area.x);
    let distance_right = area.right() - window_right(old_position, old_size);
    let distance_top = i64::from(old_position.y) - i64::from(area.y);
    let distance_bottom = area.bottom() - window_bottom(old_position, old_size);

    let x = if distance_right <= distance_left {
        area.right().saturating_sub(i64::from(new_size.width)) as i32
    } else {
        old_position.x
    };
    let y = if distance_bottom <= distance_top {
        area.bottom().saturating_sub(i64::from(new_size.height)) as i32
    } else {
        old_position.y
    };

    clamp_to_work_area(Point { x, y }, new_size, area)
}

pub fn is_inside_work_area(position: Point, size: Size, area: WorkArea) -> bool {
    i64::from(position.x) >= i64::from(area.x)
        && i64::from(position.y) >= i64::from(area.y)
        && window_right(position, size) <= area.right()
        && window_bottom(position, size) <= area.bottom()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRIMARY: WorkArea = WorkArea {
        x: 0,
        y: 0,
        width: 1920,
        height: 1040,
    };

    #[test]
    fn bottom_right_respects_taskbar_work_area_and_margin() {
        let point = bottom_right(
            Size {
                width: 310,
                height: 92,
            },
            PRIMARY,
        );
        assert_eq!(point, Point { x: 1598, y: 936 });
    }

    #[test]
    fn startup_capsule_does_not_reuse_expanded_window_top_left() {
        let capsule = Size {
            width: 310,
            height: 92,
        };
        let expanded_top_left = Point { x: 1488, y: 508 };

        assert_eq!(
            bottom_right_in_selected_work_area(expanded_top_left, capsule, &[PRIMARY]),
            Some(Point { x: 1598, y: 936 })
        );
    }

    #[test]
    fn startup_capsule_keeps_the_monitor_selected_by_the_saved_position() {
        let left = WorkArea {
            x: -1600,
            y: 0,
            width: 1600,
            height: 860,
        };
        let capsule = Size {
            width: 310,
            height: 92,
        };
        let saved_on_left = Point { x: -1450, y: 400 };

        assert_eq!(
            bottom_right_in_selected_work_area(saved_on_left, capsule, &[PRIMARY, left]),
            Some(Point { x: -322, y: 756 })
        );
    }

    #[test]
    fn restored_window_on_removed_monitor_returns_to_nearest_area() {
        let size = Size {
            width: 310,
            height: 92,
        };
        let saved = Point { x: 2450, y: 440 };
        let selected = select_work_area(saved, size, &[PRIMARY]).expect("应选择现存显示器");
        let clamped = clamp_to_work_area(saved, size, selected);
        assert_eq!(clamped.x, 1598);
        assert!(is_inside_work_area(clamped, size, PRIMARY));
    }

    #[test]
    fn negative_coordinate_monitor_is_selected_by_intersection() {
        let left = WorkArea {
            x: -1600,
            y: 0,
            width: 1600,
            height: 860,
        };
        let size = Size {
            width: 310,
            height: 92,
        };
        let position = Point { x: -500, y: 400 };
        assert_eq!(
            select_work_area(position, size, &[PRIMARY, left]),
            Some(left)
        );
    }

    #[test]
    fn expanding_a_right_bottom_capsule_grows_inward() {
        let capsule = Size {
            width: 310,
            height: 92,
        };
        let expanded = Size {
            width: 420,
            height: 520,
        };
        let position = bottom_right(capsule, PRIMARY);
        let next = resize_preserving_nearest_edges(position, capsule, expanded, PRIMARY);
        assert_eq!(next, Point { x: 1488, y: 508 });
        assert!(is_inside_work_area(next, expanded, PRIMARY));
    }

    #[test]
    fn oversized_window_uses_work_area_origin_without_overflowing_math() {
        let huge = Size {
            width: 3000,
            height: 2000,
        };
        assert_eq!(
            clamp_to_work_area(Point { x: 500, y: 500 }, huge, PRIMARY),
            Point { x: 0, y: 0 }
        );
    }
}
