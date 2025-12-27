use axum::{response::Html, routing::get, Router};
use hypertext::{prelude::*, Raw};

use crate::db::DbPool;
use crate::tasks::{get_demo_tasks, render_task_editor};

pub fn router() -> Router<DbPool> {
    Router::new().route("/tasks/edit", get(tasks_edit_all))
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
