//! The column header: columns by width, alignment with the rows, sorting.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Role, TestAppContext, px};

use crate::{click, drag_divider, node, open_app, row, rows_now, sized_tree};

/// Two folders and four files of known sizes and types. By name: alpha,
/// zeta, b.txt, c.md, d.rs, e.txt.
const SIZED: &[(&str, u64)] = &[
    ("zeta/z.bin", 100),
    ("alpha/a.bin", 5_000),
    ("e.txt", 40),
    ("c.md", 3_000),
    ("b.txt", 10),
    ("d.rs", 700),
];

#[gpui_kit::test]
fn size_and_type_columns_show_from_their_widths(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, sized_tree(SIZED));
    let columns = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let header = (
                window.try_find("header-type").is_some(),
                window.try_find("header-size").is_some(),
            );
            let cells = (window.find_all("type").len(), window.find_all("size").len());
            (header, cells)
        })
        .unwrap()
    };

    // 300 px: Name and Size.
    assert_eq!(columns(cx), ((false, true), (0, 6)));
    // 280 px: Name alone.
    drag_divider(cx, handle, 280.);
    assert_eq!(columns(cx), ((false, false), (0, 0)));
    // 420 px: all three.
    drag_divider(cx, handle, 420.);
    assert_eq!(columns(cx), ((true, true), (6, 6)));
    assert_eq!(
        cx.update_window(handle, |_, window, _| {
            window.find("header-name").label().map(str::to_owned)
        })
        .unwrap()
        .as_deref(),
        Some("Name")
    );
}

#[gpui_kit::test]
fn header_cells_line_up_with_the_row_cells(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, sized_tree(SIZED));
    let (alpha, d) = (node(cx, &view, "alpha"), node(cx, &view, "d.rs"));
    drag_divider(cx, handle, 480.);

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let header = window.find("header").bounds();
        assert_eq!(header.size.height, px(28.));
        assert_eq!(window.find("header").role(), Some(Role::Row));
        for id in [alpha, d] {
            let (kind, size) = (
                window.within(row(id)).find("type").bounds(),
                window.within(row(id)).find("size").bounds(),
            );
            let (kind_header, size_header) = (
                window.find("header-type").bounds(),
                window.find("header-size").bounds(),
            );
            assert_eq!(
                (kind.left(), kind.right()),
                (kind_header.left(), kind_header.right())
            );
            assert_eq!(
                (size.left(), size.right()),
                (size_header.left(), size_header.right())
            );
        }
        // The Name label starts where the rows' content does: at the
        // chevron's target, 8 px in.
        let name = window.find("header-name").bounds();
        let chevron = window.within(row(alpha)).find("chevron").bounds();
        assert_eq!(name.left(), chevron.left());
        assert_eq!(name.left() - header.left(), px(8.));
        let ext = window.within(row(d)).find("type");
        assert!(ext.visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn clicking_a_header_sorts_by_it_and_again_flips_the_direction(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, sized_tree(SIZED));
    drag_divider(cx, handle, 480.);
    let names = ["alpha", "zeta", "b.txt", "c.md", "d.rs", "e.txt"];
    assert_eq!(rows_now(cx, handle), names);

    // Name, the starting sort, flips: folders still come first.
    click(cx, handle, "header-name");
    assert_eq!(
        rows_now(cx, handle),
        ["zeta", "alpha", "e.txt", "d.rs", "c.md", "b.txt"]
    );

    // Size starts ascending: alpha (5,000) after zeta (100).
    click(cx, handle, "header-size");
    assert_eq!(
        rows_now(cx, handle),
        ["zeta", "alpha", "b.txt", "e.txt", "d.rs", "c.md"]
    );
    click(cx, handle, "header-size");
    assert_eq!(
        rows_now(cx, handle),
        ["alpha", "zeta", "c.md", "d.rs", "e.txt", "b.txt"]
    );

    // Type: extension, then name, ascending; folders first.
    click(cx, handle, "header-type");
    assert_eq!(
        rows_now(cx, handle),
        ["alpha", "zeta", "c.md", "d.rs", "b.txt", "e.txt"]
    );
    click(cx, handle, "header-type");
    // Folders have no type, and the name tiebreak stays ascending.
    assert_eq!(
        rows_now(cx, handle),
        ["alpha", "zeta", "b.txt", "e.txt", "d.rs", "c.md"]
    );
}

#[gpui_kit::test]
fn an_arrow_marks_the_sorted_column_and_its_direction(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, sized_tree(SIZED));
    // Which arrow each of Name and Size shows.
    let arrows = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            ["header-name", "header-size"].map(|id| {
                ["sort-ascending", "sort-descending"]
                    .into_iter()
                    .find(|arrow| window.within(id).try_find(*arrow).is_some())
            })
        })
        .unwrap()
    };

    assert_eq!(arrows(cx), [Some("sort-ascending"), None]);
    click(cx, handle, "header-size");
    assert_eq!(arrows(cx), [None, Some("sort-ascending")]);
    click(cx, handle, "header-size");
    assert_eq!(arrows(cx), [None, Some("sort-descending")]);
}
