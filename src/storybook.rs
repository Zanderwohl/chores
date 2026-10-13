use axum::{response::Html, routing::get, Router};
use hypertext::{prelude::*, Raw};

use crate::db::DbPool;
use crate::photos::render_photo_pagination;
use crate::tasks::{get_demo_tasks, render_task_editor};

pub fn router() -> Router<DbPool> {
    Router::new()
        .route("/tasks/edit", get(tasks_edit_all))
        .route("/navigator", get(navigator))
}

// GET /storybook/tasks/edit - Show all demo tasks in a grid
async fn tasks_edit_all() -> Html<String> {
    let tasks = get_demo_tasks();
    let tasks_guard = tasks.lock().unwrap();

    // Get tasks in order
    let mut task_ids: Vec<&String> = tasks_guard.keys().collect();
    task_ids.sort();

    let task_editors: Vec<String> = task_ids
        .iter()
        .filter_map(|id| tasks_guard.get(*id).map(render_task_editor))
        .collect();

    let editors_html = task_editors.join("\n");

    let html = maud! {
        !DOCTYPE
        html {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { "Task Editor - Storybook" }
                link rel="stylesheet" href="/static/system.css";
                link rel="stylesheet" href="/static/app.css";
                script src="https://unpkg.com/htmx.org@2.0.4" {}
            }
            body {
                h1 { "Task Editor Storybook" }
                p { "Edit forms for tasks with different schedule types:" }

                div .task-grid {
                    (Raw::dangerously_create(&editors_html))
                }
            }
        }
    };

    Html(html.render().into_inner())
}

// GET /storybook/navigator - Show the photo paginator at different page counts
// and positions. Buttons target #photo-list, which doesn't exist here, so
// clicking them does nothing.
async fn navigator() -> Html<String> {
    const PER_PAGE: i64 = 20;
    let cases: &[(i64, &[i64])] = &[
        (5, &[1, 3, 5]),
        (10, &[1, 6, 10]),
        (12, &[1, 3, 4, 7, 12]),
        (100, &[1, 2, 3, 4, 50, 91, 92, 93, 99, 100]),
    ];

    let examples_html: String = cases
        .iter()
        .flat_map(|&(total_pages, currents)| {
            currents.iter().map(move |&current| {
                let total_count = total_pages * PER_PAGE;
                format!(
                    r#"<section class="storybook-example"><h3>Page {} of {}</h3>{}</section>"#,
                    current,
                    total_pages,
                    render_photo_pagination(current, total_pages, PER_PAGE, total_count)
                )
            })
        })
        .collect::<Vec<_>>()
        .join("\n");

    let html = maud! {
        !DOCTYPE
        html {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { "Navigator - Storybook" }
                link rel="stylesheet" href="/static/system.css";
                link rel="stylesheet" href="/static/app.css";
            }
            body {
                h1 { "Page Navigator Storybook" }
                p { "Shows at most 10 page numbers: up to 2 before the current page and the rest after it." }
                (Raw::dangerously_create(&examples_html))
            }
        }
    };

    Html(html.render().into_inner())
}
