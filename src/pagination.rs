//! Shared page-number windowing for the paginated lists.

/// Most page-number buttons a paginator shows at once.
const MAX_PAGES: i64 = 10;
/// Most page numbers shown before the current page, unless the window is
/// pushed left by running out of pages at the end.
const MAX_PREVIOUS: i64 = 2;

/// The range of page numbers to show: up to `MAX_PREVIOUS` before
/// the current page and the rest after it. Near the last page, the leftover
/// slots move to the left so the window stays full.
pub fn window(current_page: i64, total_pages: i64) -> std::ops::RangeInclusive<i64> {
    let end = (current_page - MAX_PREVIOUS + MAX_PAGES - 1)
        .max(MAX_PAGES)
        .min(total_pages);
    let start = (end - MAX_PAGES + 1).max(1);
    start..=end
}

#[cfg(test)]
mod tests {
    fn window(current: i64, total: i64) -> (i64, i64) {
        let w = super::window(current, total);
        (*w.start(), *w.end())
    }

    #[test]
    fn test_pagination_window_fewer_pages_than_slots() {
        assert_eq!(window(1, 5), (1, 5));
        assert_eq!(window(5, 5), (1, 5));
        assert_eq!(window(4, 10), (1, 10));
    }

    #[test]
    fn test_pagination_window_near_start() {
        assert_eq!(window(1, 100), (1, 10));
        assert_eq!(window(2, 100), (1, 10));
        assert_eq!(window(3, 100), (1, 10));
    }

    #[test]
    fn test_pagination_window_two_previous() {
        assert_eq!(window(4, 100), (2, 11));
        assert_eq!(window(50, 100), (48, 57));
    }

    #[test]
    fn test_pagination_window_near_end() {
        assert_eq!(window(92, 100), (90, 99));
        assert_eq!(window(93, 100), (91, 100));
        assert_eq!(window(98, 100), (91, 100));
        assert_eq!(window(100, 100), (91, 100));
    }
}
