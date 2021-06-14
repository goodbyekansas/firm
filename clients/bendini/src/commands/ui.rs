use std::{collections::HashMap, fmt::Display, sync::Arc};

use cursive::{
    traits::{Boxable, Nameable},
    views::{LinearLayout, PaddedView, Panel, SelectView},
    Cursive,
};

use cursive_tree_view::{Placement, TreeView};
use firm_types::{
    functions::{registry_client::RegistryClient, Filters, Function, Ordering, OrderingKey},
    tonic::Request,
};
use tonic_middleware::HttpStatusInterceptor;

use crate::error::BendiniError;

#[derive(Debug)]
pub struct TreeEntry {
    function: Arc<Function>,
    display: String,
}

impl Display for TreeEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display)
    }
}

pub async fn run(mut client: RegistryClient<HttpStatusInterceptor>) -> Result<(), BendiniError> {
    let mut siv = cursive::default();

    // We can quit by pressing `q`
    siv.add_global_callback('q', Cursive::quit);

    siv.load_toml(
        r##"
# Every field in a theme file is optional.

shadow = true
borders = "outset" # Alternatives are "none" and "simple"

# Base colors are red, green, blue,
# cyan, magenta, yellow, white and black.
[colors]
  # There are 3 ways to select a color:
  # - The 16 base colors are selected by name:
  #       "blue", "light red", "magenta", ...
  # - Low-resolution colors use 3 characters, each <= 5:
  #       "541", "003", ...
  # - Full-resolution colors start with '#' and can be 3 or 6 hex digits:
  #       "#1A6", "#123456", ...

  # If the value is an array, the first valid
  # and supported color will be used.
  background = [ "#0000aa", "blue" ]

  # If the terminal doesn't support custom color (like the linux TTY),
  # non-base colors will be skipped.
  shadow     = "black"
  view       = "grey"

  # An array with a single value has the same effect as a simple value.
  primary   = "light grey"
  secondary = "black"
  tertiary  = "magenta"

  # Hex values can use lower or uppercase.
  # (base color MUST be lowercase)
  title_primary   = "light grey"
  title_secondary = "white"

  # Lower precision values can use only 3 digits.
  highlight          = "bright black"
  highlight_inactive = "green"
"##,
    )
    .unwrap();

    let mut tree_view = TreeView::new();
    let list_request = Filters {
        name: None,
        metadata: HashMap::new(),
        order: Some(Ordering {
            limit: 25,
            offset: 0,
            reverse: false,
            key: OrderingKey::NameVersion as i32,
        }),
        version_requirement: None,
    };

    let functions = client
        .list(Request::new(list_request))
        .await?
        .into_inner()
        .functions;

    functions.into_iter().for_each(|f| {
        tree_view.insert_item(
            TreeEntry {
                display: f.name.clone(),
                function: Arc::new(f),
            },
            Placement::After,
            0,
        );
    });

    /*tree_view.insert_item(f.version, Placement::LastChild, 1);

    if !f.metadata.is_empty() {
        tree_view.insert_item(String::from("metadata"), Placement::LastChild, 1);
        f.metadata.into_iter().for_each(|(k, v)| {
            tree_view.insert_item(format!("{}:{}", k, v), Placement::LastChild, 2);
        });
    }*/

    tree_view.set_on_collapse(|siv: &mut Cursive, row, is_collapsed, children| {
        if !is_collapsed && children == 0 {
            siv.call_on_name("tree", move |tree: &mut TreeView<TreeEntry>| {
                if let Some(f) = tree.borrow_item(row) {
                    let vers = f.function.version.clone();
                    let fun = Arc::clone(&f.function);
                    tree.insert_item(
                        TreeEntry {
                            display: vers,
                            function: fun,
                        },
                        Placement::LastChild,
                        row,
                    );
                }
            });
        }
    });

    siv.add_layer(
        LinearLayout::horizontal()
            .child(Panel::new(PaddedView::lrtb(
                2,
                2,
                0,
                0,
                SelectView::new()
                    .item("Functions", 1)
                    .item("Authentication", 2)
                    .full_height(),
            )))
            .child(PaddedView::lrtb(2, 2, 0, 0, tree_view.with_name("tree"))),
    );

    // Run the event loop
    siv.run();
    Ok(())
}
