use axum::{
    extract::{Path, Query, State},
    response::Html,
    routing::{get, post},
    Form, Router,
};
use chrono::{DateTime, Datelike, Duration, Local, NaiveTime, Utc};
use hypertext::{prelude::*, Raw};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use crate::db::{self, DbPool};
use crate::schedule::{DaysOfWeek, Monthwise, NDays, NWeeks, ScheduleKind, WeeksOfMonth};

// Shared state for demo tasks (in-memory)
pub type DemoTasksMap = Arc<Mutex<HashMap<String, DemoTask>>>;
pub static DEMO_TASKS: OnceLock<DemoTasksMap> = OnceLock::new();

pub fn get_demo_tasks() -> &'static DemoTasksMap {
    DEMO_TASKS.get_or_init(|| {
        let mut tasks = HashMap::new();

        tasks.insert(
            "demo-1".to_string(),
            DemoTask {
                id: "demo-1".to_string(),
                name: "Water Plants".to_string(),
                details: "Water all indoor plants, including the fern in the living room."
                    .to_string(),
                schedule_kind: ScheduleKind::NDays,
                n_days: NDays {
                    days: 3,
                    time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                },
                n_weeks: default_n_weeks(),
                monthwise: default_monthwise(),
                weeks_of_month: default_weeks_of_month(),
            },
        );

        tasks.insert(
            "demo-2".to_string(),
            DemoTask {
                id: "demo-2".to_string(),
                name: "Take Out Trash".to_string(),
                details: "Take recycling and garbage bins to the curb.".to_string(),
                schedule_kind: ScheduleKind::NWeeks,
                n_days: default_n_days(),
                n_weeks: NWeeks {
                    weeks: 1,
                    sub_schedule: DaysOfWeek {
                        sunday: false,
                        monday: true,
                        tuesday: false,
                        wednesday: false,
                        thursday: true,
                        friday: false,
                        saturday: false,
                        time: NaiveTime::from_hms_opt(7, 0, 0).unwrap(),
                    },
                },
                monthwise: default_monthwise(),
                weeks_of_month: default_weeks_of_month(),
            },
        );

        tasks.insert(
            "demo-3".to_string(),
            DemoTask {
                id: "demo-3".to_string(),
                name: "Pay Rent".to_string(),
                details: "Transfer rent payment to landlord.".to_string(),
                schedule_kind: ScheduleKind::Monthwise,
                n_days: default_n_days(),
                n_weeks: default_n_weeks(),
                monthwise: Monthwise {
                    days: vec![1],
                    time: NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
                },
                weeks_of_month: default_weeks_of_month(),
            },
        );

        tasks.insert(
            "demo-4".to_string(),
            DemoTask {
                id: "demo-4".to_string(),
                name: "Team Meeting".to_string(),
                details: "Attend bi-weekly team standup on the 1st and 3rd Tuesday.".to_string(),
                schedule_kind: ScheduleKind::WeeksOfMonth,
                n_days: default_n_days(),
                n_weeks: default_n_weeks(),
                monthwise: default_monthwise(),
                weeks_of_month: WeeksOfMonth {
                    weeks: vec![1, 3],
                    sub_schedule: DaysOfWeek {
                        sunday: false,
                        monday: false,
                        tuesday: true,
                        wednesday: false,
                        thursday: false,
                        friday: false,
                        saturday: false,
                        time: NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
                    },
                },
            },
        );

        Arc::new(Mutex::new(tasks))
    })
}

/// Check if an ID is a demo task ID
fn is_demo_id(id: &str) -> bool {
    id.starts_with("demo-")
}

pub fn router() -> Router<DbPool> {
    Router::new()
        .route("/", get(tasks_index))
        .route("/list", get(tasks_list))
        .route("/new", get(new_task_modal).post(create_task))
        .route("/new/schedule-type", post(new_task_schedule_type))
        .route("/{id}/edit", get(task_edit))
        .route("/{id}/edit-modal", get(task_edit_modal))
        .route("/{id}", get(task_show).post(save_task))
        .route("/{id}/schedule-type", post(change_schedule_type))
        .route("/{id}/complete", post(complete_task))
        .route("/{id}/completions/{completion_id}", axum::routing::delete(delete_completion))
}

// POST /tasks/:id/complete - Mark a task as complete
async fn complete_task(State(pool): State<DbPool>, Path(id): Path<String>) -> Html<String> {
    // Add completion record
    if let Err(e) = db::add_completion(&pool, &id).await {
        eprintln!("Error adding completion: {}", e);
    }

    // Re-render the entire homepage
    homepage(State(pool)).await
}

// GET /tasks/:id - Show page for a single task
async fn task_show(State(pool): State<DbPool>, Path(id): Path<String>) -> Html<String> {
    let task = if is_demo_id(&id) {
        let tasks = get_demo_tasks();
        let tasks_guard = tasks.lock().unwrap();
        tasks_guard.get(&id).cloned()
    } else {
        if let Ok(task_id) = id.parse::<i64>() {
            db::get_task(&pool, task_id).await.ok().flatten()
        } else {
            None
        }
    };

    let Some(task) = task else {
        return Html(format!(
            "<!DOCTYPE html><html><head><title>Not Found</title></head><body><h1>Task '{}' not found</h1><a href=\"/tasks\">Back to Tasks</a></body></html>",
            id
        ));
    };

    // Get all completions for calendar and list
    let completions = db::get_all_completions(&pool, &id).await.unwrap_or_default();

    Html(render_task_show_page(&task, &completions))
}

// DELETE /tasks/:id/completions/:completion_id - Delete a completion
async fn delete_completion(
    State(pool): State<DbPool>,
    Path((task_id, completion_id)): Path<(String, i64)>,
) -> Html<String> {
    if let Err(e) = db::delete_completion(&pool, completion_id).await {
        eprintln!("Error deleting completion: {}", e);
    }

    // Re-render the task show page
    task_show(State(pool), Path(task_id)).await
}

// Helper struct to hold task with its completion status
struct TaskWithStatus {
    task: DemoTask,
    is_completed: bool,
}

// GET / - Homepage with task cards
pub async fn homepage(State(pool): State<DbPool>) -> Html<String> {
    // Collect all tasks from database only (demo tasks are excluded from index)
    let all_tasks: Vec<DemoTask> = db::get_all_tasks(&pool).await.unwrap_or_default();

    // Check completion status for each task
    let mut tasks_with_status: Vec<TaskWithStatus> = Vec::new();
    for task in all_tasks {
        let is_completed = if let Ok(Some(completion_time)) = db::get_latest_completion(&pool, &task.id).await {
            // Task is completed if completion is after the most recent due date
            completion_time > task.most_recent_due_date()
        } else {
            false
        };
        tasks_with_status.push(TaskWithStatus { task, is_completed });
    }

    // Sort all tasks by next due date
    tasks_with_status.sort_by(|a, b| a.task.next_due_date().cmp(&b.task.next_due_date()));

    // Categorize tasks
    let mut due_tasks = Vec::new();
    let mut alerting_tasks = Vec::new();
    let mut completed_tasks = Vec::new();
    let mut other_tasks = Vec::new();

    for ts in tasks_with_status {
        if ts.is_completed {
            completed_tasks.push(ts.task);
        } else if ts.task.is_due() {
            due_tasks.push(ts.task);
        } else if ts.task.is_alerting() {
            alerting_tasks.push(ts.task);
        } else {
            other_tasks.push(ts.task);
        }
    }

    let html = maud! {
        !DOCTYPE
        html {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { "Chores" }
                link rel="stylesheet" href="/static/system.css";
                link rel="stylesheet" href="/static/app.css";
                script src="https://unpkg.com/htmx.org@2.0.4" {}
            }
            body {
                div .homepage id="homepage" {
                    h1 { "Chores" }

                    @if !due_tasks.is_empty() {
                        section .task-section {
                            h2 { "Due Tasks" }
                            div .task-card-grid {
                                @for task in &due_tasks {
                                    (Raw::dangerously_create(&render_task_card(task, "due")))
                                }
                            }
                        }
                    }

                    @if !alerting_tasks.is_empty() {
                        section .task-section {
                            h2 { "Upcoming Tasks" }
                            div .task-card-grid {
                                @for task in &alerting_tasks {
                                    (Raw::dangerously_create(&render_task_card(task, "alerting")))
                                }
                            }
                        }
                    }

                    @if !completed_tasks.is_empty() {
                        section .task-section {
                            h2 { "Completed" }
                            div .task-card-grid {
                                @for task in &completed_tasks {
                                    (Raw::dangerously_create(&render_task_card(task, "completed")))
                                }
                            }
                        }
                    }

                    @if !other_tasks.is_empty() {
                        section .task-section {
                            h2 { "Other Tasks" }
                            div .task-card-grid {
                                @for task in &other_tasks {
                                    (Raw::dangerously_create(&render_task_card(task, "normal")))
                                }
                            }
                        }
                    }

                    @if due_tasks.is_empty() && alerting_tasks.is_empty() && completed_tasks.is_empty() && other_tasks.is_empty() {
                        div .empty-state {
                            p { "No tasks yet!" }
                            a href="/tasks" { "Go to Tasks →" }
                        }
                    }

                    div .homepage-footer {
                        a href="/tasks" { "Manage Tasks →" }
                    }
                }
            }
        }
    };

    Html(html.render().into_inner())
}

fn render_task_card(task: &DemoTask, status: &str) -> String {
    let status_class = format!("task-card task-card-{}", status);
    let due_str = task.time_as_readable_string();
    let complete_url = format!("/tasks/{}/complete", task.id);
    let show_url = format!("/tasks/{}", task.id);
    let is_completed = status == "completed";

    // Complete button (only show if not already completed)
    let complete_button = if is_completed {
        r#"<div class="task-card-completed-label">✓ Done</div>"#.to_string()
    } else {
        format!(
            r##"<button class="btn task-card-complete-btn" hx-post="{}" hx-target="#homepage" hx-swap="outerHTML">Complete</button>"##,
            complete_url
        )
    };

    maud! {
        div class=(status_class) {
            a .task-card-title href=(show_url) { (task.name) }
            @if !task.details.is_empty() {
                div .task-card-description { (task.details) }
            }
            (Raw::dangerously_create(&complete_button))
            div .task-card-due { (due_str) }
        }
    }
    .render()
    .into_inner()
}

fn render_task_show_page(task: &DemoTask, completions: &[db::CompletionRecord]) -> String {
    use chrono::Datelike;

    let schedule_type_label = match task.schedule_kind {
        ScheduleKind::NDays => format!("Every {} day(s)", task.n_days.days),
        ScheduleKind::NWeeks => {
            let days: Vec<&str> = [
                ("Sun", task.n_weeks.sub_schedule.sunday),
                ("Mon", task.n_weeks.sub_schedule.monday),
                ("Tue", task.n_weeks.sub_schedule.tuesday),
                ("Wed", task.n_weeks.sub_schedule.wednesday),
                ("Thu", task.n_weeks.sub_schedule.thursday),
                ("Fri", task.n_weeks.sub_schedule.friday),
                ("Sat", task.n_weeks.sub_schedule.saturday),
            ]
            .iter()
            .filter(|(_, active)| *active)
            .map(|(name, _)| *name)
            .collect();
            format!("Every {} week(s) on {}", task.n_weeks.weeks, days.join(", "))
        }
        ScheduleKind::Monthwise => {
            let days_str = task.monthwise.days.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(", ");
            format!("Monthly on day(s) {}", days_str)
        }
        ScheduleKind::WeeksOfMonth => {
            let weeks_str = task.weeks_of_month.weeks.iter().map(|w| {
                match w {
                    1 => "1st",
                    2 => "2nd",
                    3 => "3rd",
                    4 => "4th",
                    5 => "5th",
                    _ => "?",
                }
            }).collect::<Vec<_>>().join(", ");
            let days: Vec<&str> = [
                ("Sun", task.weeks_of_month.sub_schedule.sunday),
                ("Mon", task.weeks_of_month.sub_schedule.monday),
                ("Tue", task.weeks_of_month.sub_schedule.tuesday),
                ("Wed", task.weeks_of_month.sub_schedule.wednesday),
                ("Thu", task.weeks_of_month.sub_schedule.thursday),
                ("Fri", task.weeks_of_month.sub_schedule.friday),
                ("Sat", task.weeks_of_month.sub_schedule.saturday),
            ]
            .iter()
            .filter(|(_, active)| *active)
            .map(|(name, _)| *name)
            .collect();
            format!("{} week(s) on {}", weeks_str, days.join(", "))
        }
    };

    let next_due_str = task.time_as_readable_string();
    let calendar_html = render_calendar(task, completions);
    let completions_html = render_completions_list(&task.id, completions);
    let edit_url = format!("/tasks/{}/edit-modal", task.id);

    let edit_button = format!(
        r##"<button class="btn" hx-get="{}" hx-target="#modal-container" hx-swap="innerHTML">Edit</button>"##,
        edit_url
    );

    maud! {
        !DOCTYPE
        html {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (task.name) " - Chores" }
                link rel="stylesheet" href="/static/system.css";
                link rel="stylesheet" href="/static/app.css";
                script src="https://unpkg.com/htmx.org@2.0.4" {}
            }
            body {
                div .task-show-page id="task-show-page" {
                    div .task-show-header {
                        a href="/" { "← Home" }
                        " | "
                        a href="/tasks" { "Tasks" }
                    }

                    div .task-show-title-row {
                        h1 { (task.name) }
                        (Raw::dangerously_create(&edit_button))
                    }

                    @if !task.details.is_empty() {
                        div .task-show-details {
                            p { (task.details) }
                        }
                    }

                    div .task-show-info {
                        div .task-show-info-row {
                            strong { "Schedule: " }
                            span { (schedule_type_label) }
                        }
                        div .task-show-info-row {
                            strong { "Next Due: " }
                            span { (next_due_str) }
                        }
                    }

                    section .task-show-section {
                        h2 { "Calendar" }
                        (Raw::dangerously_create(&calendar_html))
                    }

                    section .task-show-section {
                        h2 { "Completions" }
                        (Raw::dangerously_create(&completions_html))
                    }

                    // Modal container for edit
                    div #modal-container {}
                }
            }
        }
    }
    .render()
    .into_inner()
}

fn render_calendar(task: &DemoTask, completions: &[db::CompletionRecord]) -> String {
    use chrono::{Datelike, NaiveDate, Weekday};

    let now = Local::now();
    let year = now.year();
    let month = now.month();

    // Get first day of month and number of days
    let first_of_month = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
    let days_in_month = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)
    }
    .unwrap()
    .signed_duration_since(first_of_month)
    .num_days() as u32;

    let first_weekday = first_of_month.weekday();
    let start_offset = match first_weekday {
        Weekday::Sun => 0,
        Weekday::Mon => 1,
        Weekday::Tue => 2,
        Weekday::Wed => 3,
        Weekday::Thu => 4,
        Weekday::Fri => 5,
        Weekday::Sat => 6,
    };

    let month_name = match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "",
    };

    // Calculate due dates for this month
    let mut due_dates: std::collections::HashMap<u32, chrono::NaiveTime> = std::collections::HashMap::new();

    for day in 1..=days_in_month {
        let date = NaiveDate::from_ymd_opt(year, month, day).unwrap();
        if is_due_on_date(task, date) {
            let time = get_due_time(task, date);
            due_dates.insert(day, time);
        }
    }

    // Build calendar grid
    let mut cells = String::new();

    // Header row
    cells.push_str(r#"<div class="calendar-header-row">"#);
    for day_name in &["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"] {
        cells.push_str(&format!(r#"<div class="calendar-header-cell">{}</div>"#, day_name));
    }
    cells.push_str("</div>");

    // Day cells
    let mut cell_count = 0;
    cells.push_str(r#"<div class="calendar-row">"#);

    // Empty cells before first day
    for _ in 0..start_offset {
        cells.push_str(r#"<div class="calendar-cell calendar-cell-empty"></div>"#);
        cell_count += 1;
    }

    for day in 1..=days_in_month {
        if cell_count > 0 && cell_count % 7 == 0 {
            cells.push_str("</div>");
            cells.push_str(r#"<div class="calendar-row">"#);
        }

        let date = NaiveDate::from_ymd_opt(year, month, day).unwrap();
        let is_today = date == now.date_naive();

        let mut cell_class = "calendar-cell".to_string();
        if is_today {
            cell_class.push_str(" calendar-cell-today");
        }

        let mut content = format!(r#"<span class="calendar-day-number">{}</span>"#, day);

        // Check if due on this day
        if let Some(time) = due_dates.get(&day) {
            content.push_str(&format!(
                r#"<div class="calendar-due">Due at {}</div>"#,
                time.format("%H:%M")
            ));

            // Check if completed after this due date but before next due
            let due_datetime = date
                .and_time(*time)
                .and_local_timezone(Local)
                .unwrap()
                .with_timezone(&Utc);

            // Find next due date after this one
            let next_due = find_next_due_after(task, due_datetime);

            let is_completed = completions.iter().any(|c| {
                c.completed_at > due_datetime && c.completed_at <= next_due
            });

            if is_completed {
                content.push_str(r#"<div class="calendar-completed">✓ Completed</div>"#);
            }
        }

        cells.push_str(&format!(
            r#"<div class="{}">{}</div>"#,
            cell_class, content
        ));
        cell_count += 1;
    }

    // Fill remaining cells
    while cell_count % 7 != 0 {
        cells.push_str(r#"<div class="calendar-cell calendar-cell-empty"></div>"#);
        cell_count += 1;
    }
    cells.push_str("</div>");

    format!(
        r#"<div class="calendar">
            <div class="calendar-title">{} {}</div>
            <div class="calendar-grid">{}</div>
        </div>"#,
        month_name, year, cells
    )
}

fn is_due_on_date(task: &DemoTask, date: chrono::NaiveDate) -> bool {
    use chrono::Datelike;

    match task.schedule_kind {
        ScheduleKind::NDays => {
            // For NDays, calculate based on interval from today
            // A task is due every N days, so we check if the date is N days apart from today
            let today = Local::now().date_naive();
            let days_diff = (date - today).num_days().abs();
            days_diff % (task.n_days.days as i64) == 0
        }
        ScheduleKind::NWeeks => {
            let weekday = date.weekday();
            task.n_weeks.sub_schedule.active(weekday)
        }
        ScheduleKind::Monthwise => {
            let day = date.day() as i32;
            task.monthwise.days.contains(&day)
        }
        ScheduleKind::WeeksOfMonth => {
            let weekday = date.weekday();
            let week_num = ((date.day() - 1) / 7 + 1) as i32;
            task.weeks_of_month.sub_schedule.active(weekday) && task.weeks_of_month.weeks.contains(&week_num)
        }
    }
}

fn get_due_time(task: &DemoTask, _date: chrono::NaiveDate) -> chrono::NaiveTime {
    match task.schedule_kind {
        ScheduleKind::NDays => task.n_days.time,
        ScheduleKind::NWeeks => task.n_weeks.sub_schedule.time,
        ScheduleKind::Monthwise => task.monthwise.time,
        ScheduleKind::WeeksOfMonth => task.weeks_of_month.sub_schedule.time,
    }
}

fn find_next_due_after(task: &DemoTask, after: DateTime<Utc>) -> DateTime<Utc> {
    use chrono::Datelike;

    let local_after: DateTime<Local> = after.into();

    // Look ahead up to 60 days for the next due date
    for days_ahead in 1..=60 {
        let check_date = (local_after + Duration::days(days_ahead)).date_naive();
        if is_due_on_date(task, check_date) {
            let time = get_due_time(task, check_date);
            return check_date
                .and_time(time)
                .and_local_timezone(Local)
                .unwrap()
                .with_timezone(&Utc);
        }
    }

    // Default: 60 days from now
    after + Duration::days(60)
}

fn render_completions_list(task_id: &str, completions: &[db::CompletionRecord]) -> String {
    if completions.is_empty() {
        return maud! {
            div .completions-empty {
                p { "No completions recorded yet." }
            }
        }
        .render()
        .into_inner();
    }

    let items: Vec<String> = completions
        .iter()
        .map(|c| {
            let local: DateTime<Local> = c.completed_at.into();
            let formatted = local.format("%A, %B %-d, %Y at %H:%M").to_string();
            let delete_url = format!("/tasks/{}/completions/{}", task_id, c.id);

            format!(
                r##"<li class="completion-item">
                    <span class="completion-date">{}</span>
                    <button class="btn completion-delete" hx-delete="{}" hx-target="#task-show-page" hx-swap="outerHTML" hx-confirm="Delete this completion?">×</button>
                </li>"##,
                formatted, delete_url
            )
        })
        .collect();

    maud! {
        ul .completions-list {
            (Raw::dangerously_create(&items.join("\n")))
        }
    }
    .render()
    .into_inner()
}

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default = "default_sort")]
    pub sort: String,
}

fn default_sort() -> String {
    "name".to_string()
}

// GET /tasks - Show the task index page
async fn tasks_index(State(pool): State<DbPool>, Query(query): Query<ListQuery>) -> Html<String> {
    let list_html = render_task_list(&pool, &query.sort).await;

    let html = maud! {
        !DOCTYPE
        html {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { "Tasks - Chores" }
                link rel="stylesheet" href="/static/system.css";
                link rel="stylesheet" href="/static/app.css";
                script src="https://unpkg.com/htmx.org@2.0.4" {}
            }
            body {
                div .tasks-page {
                    div .tasks-page-header {
                        a href="/" { "← Home" }
                    }

                    h1 { "Tasks" }

                    // Sorting controls and New Task button
                    div .list-controls {
                        div .list-controls-left {
                            label for="sort-select" { "Sort by: " }
                            (Raw::dangerously_create(&render_sort_select(&query.sort)))
                        }
                        (Raw::dangerously_create(
                            r##"<button class="btn" hx-get="/tasks/new" hx-target="#modal-container" hx-swap="innerHTML">New Task</button>"##
                        ))
                    }

                    // Task list container
                    div #task-list {
                        (Raw::dangerously_create(&list_html))
                    }

                    // Modal container (initially empty)
                    div #modal-container {}
                }
            }
        }
    };

    Html(html.render().into_inner())
}

// GET /tasks/list - Return just the task list (for HTMX)
async fn tasks_list(State(pool): State<DbPool>, Query(query): Query<ListQuery>) -> Html<String> {
    Html(render_task_list(&pool, &query.sort).await)
}

// GET /tasks/:id/edit - Get edit view for a single task (standalone, from saved state)
async fn task_edit(State(pool): State<DbPool>, Path(id): Path<String>) -> Html<String> {
    if is_demo_id(&id) {
        let tasks = get_demo_tasks();
        let tasks_guard = tasks.lock().unwrap();
        if let Some(task) = tasks_guard.get(&id) {
            return Html(render_task_editor(task));
        }
    } else {
        if let Ok(task_id) = id.parse::<i64>() {
            if let Ok(Some(task)) = db::get_task(&pool, task_id).await {
                return Html(render_task_editor(&task));
            }
        }
    }

    Html(format!(
        "<div class=\"window\"><div class=\"window-pane\">Task '{}' not found</div></div>",
        id
    ))
}

// GET /tasks/:id/edit-modal - Get edit view as a modal
async fn task_edit_modal(State(pool): State<DbPool>, Path(id): Path<String>) -> Html<String> {
    if is_demo_id(&id) {
        let tasks = get_demo_tasks();
        let tasks_guard = tasks.lock().unwrap();
        if let Some(task) = tasks_guard.get(&id) {
            return Html(render_task_modal(task));
        }
    } else {
        if let Ok(task_id) = id.parse::<i64>() {
            if let Ok(Some(task)) = db::get_task(&pool, task_id).await {
                return Html(render_task_modal(&task));
            }
        }
    }

    Html(format!(
        "<div class=\"modal-overlay\"><div class=\"window\"><div class=\"window-pane\">Task '{}' not found</div></div></div>",
        id
    ))
}

// Form data for the full task
#[derive(Deserialize, Debug)]
pub struct TaskForm {
    pub name: String,
    pub details: String,
    pub schedule_type: String,
    #[serde(default)]
    pub n_days_count: Option<i32>,
    #[serde(default)]
    pub n_days_time: Option<String>,
    #[serde(default)]
    pub n_weeks_count: Option<i32>,
    #[serde(default)]
    pub n_weeks_time: Option<String>,
    #[serde(default)]
    pub dow_sun: Option<String>,
    #[serde(default)]
    pub dow_mon: Option<String>,
    #[serde(default)]
    pub dow_tue: Option<String>,
    #[serde(default)]
    pub dow_wed: Option<String>,
    #[serde(default)]
    pub dow_thu: Option<String>,
    #[serde(default)]
    pub dow_fri: Option<String>,
    #[serde(default)]
    pub dow_sat: Option<String>,
    #[serde(default)]
    pub monthwise_days: Option<String>,
    #[serde(default)]
    pub monthwise_time: Option<String>,
    #[serde(default)]
    pub wom_week_1: Option<String>,
    #[serde(default)]
    pub wom_week_2: Option<String>,
    #[serde(default)]
    pub wom_week_3: Option<String>,
    #[serde(default)]
    pub wom_week_4: Option<String>,
    #[serde(default)]
    pub wom_week_5: Option<String>,
    #[serde(default)]
    pub wom_dow_sun: Option<String>,
    #[serde(default)]
    pub wom_dow_mon: Option<String>,
    #[serde(default)]
    pub wom_dow_tue: Option<String>,
    #[serde(default)]
    pub wom_dow_wed: Option<String>,
    #[serde(default)]
    pub wom_dow_thu: Option<String>,
    #[serde(default)]
    pub wom_dow_fri: Option<String>,
    #[serde(default)]
    pub wom_dow_sat: Option<String>,
    #[serde(default)]
    pub wom_time: Option<String>,
}

impl TaskForm {
    pub fn to_demo_task(&self, id: &str, base_task: &DemoTask) -> DemoTask {
        let schedule_kind = match self.schedule_type.as_str() {
            "n_days" => ScheduleKind::NDays,
            "n_weeks" => ScheduleKind::NWeeks,
            "monthwise" => ScheduleKind::Monthwise,
            "weeks_of_month" => ScheduleKind::WeeksOfMonth,
            _ => base_task.schedule_kind.clone(),
        };

        let n_days = NDays {
            days: self.n_days_count.unwrap_or(base_task.n_days.days),
            time: self
                .n_days_time
                .as_ref()
                .and_then(|t| NaiveTime::parse_from_str(t, "%H:%M").ok())
                .unwrap_or(base_task.n_days.time),
        };

        let n_weeks_time = self
            .n_weeks_time
            .as_ref()
            .and_then(|t| NaiveTime::parse_from_str(t, "%H:%M").ok())
            .unwrap_or(base_task.n_weeks.sub_schedule.time);
        let n_weeks = NWeeks {
            weeks: self.n_weeks_count.unwrap_or(base_task.n_weeks.weeks),
            sub_schedule: DaysOfWeek {
                sunday: self.dow_sun.is_some(),
                monday: self.dow_mon.is_some(),
                tuesday: self.dow_tue.is_some(),
                wednesday: self.dow_wed.is_some(),
                thursday: self.dow_thu.is_some(),
                friday: self.dow_fri.is_some(),
                saturday: self.dow_sat.is_some(),
                time: n_weeks_time,
            },
        };

        let monthwise_days = self
            .monthwise_days
            .as_ref()
            .map(|s| {
                s.split(',')
                    .filter_map(|d| d.trim().parse::<i32>().ok())
                    .collect::<Vec<_>>()
            })
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| base_task.monthwise.days.clone());
        let monthwise = Monthwise {
            days: monthwise_days,
            time: self
                .monthwise_time
                .as_ref()
                .and_then(|t| NaiveTime::parse_from_str(t, "%H:%M").ok())
                .unwrap_or(base_task.monthwise.time),
        };

        let mut wom_weeks = Vec::new();
        if self.wom_week_1.is_some() {
            wom_weeks.push(1);
        }
        if self.wom_week_2.is_some() {
            wom_weeks.push(2);
        }
        if self.wom_week_3.is_some() {
            wom_weeks.push(3);
        }
        if self.wom_week_4.is_some() {
            wom_weeks.push(4);
        }
        if self.wom_week_5.is_some() {
            wom_weeks.push(5);
        }
        if wom_weeks.is_empty() {
            wom_weeks = base_task.weeks_of_month.weeks.clone();
        }

        let wom_time = self
            .wom_time
            .as_ref()
            .and_then(|t| NaiveTime::parse_from_str(t, "%H:%M").ok())
            .unwrap_or(base_task.weeks_of_month.sub_schedule.time);
        let weeks_of_month = WeeksOfMonth {
            weeks: wom_weeks,
            sub_schedule: DaysOfWeek {
                sunday: self.wom_dow_sun.is_some(),
                monday: self.wom_dow_mon.is_some(),
                tuesday: self.wom_dow_tue.is_some(),
                wednesday: self.wom_dow_wed.is_some(),
                thursday: self.wom_dow_thu.is_some(),
                friday: self.wom_dow_fri.is_some(),
                saturday: self.wom_dow_sat.is_some(),
                time: wom_time,
            },
        };

        DemoTask {
            id: id.to_string(),
            name: self.name.clone(),
            details: self.details.clone(),
            schedule_kind,
            n_days,
            n_weeks,
            monthwise,
            weeks_of_month,
        }
    }
}

// POST /tasks/:id - Save the task
async fn save_task(
    State(pool): State<DbPool>,
    Path(id): Path<String>,
    Form(form): Form<TaskForm>,
) -> Html<String> {
    if is_demo_id(&id) {
        let tasks = get_demo_tasks();
        let mut tasks_guard = tasks.lock().unwrap();

        if let Some(existing_task) = tasks_guard.get(&id) {
            let updated_task = form.to_demo_task(&id, existing_task);
            tasks_guard.insert(id.clone(), updated_task);
        }

        if let Some(task) = tasks_guard.get(&id) {
            return Html(render_task_modal(task));
        }
    } else {
        if let Ok(task_id) = id.parse::<i64>() {
            if let Ok(Some(existing_task)) = db::get_task(&pool, task_id).await {
                let updated_task = form.to_demo_task(&id, &existing_task);
                if let Ok(_) = db::save_task(&pool, &updated_task).await {
                    if let Ok(Some(saved_task)) = db::get_task(&pool, task_id).await {
                        return Html(render_task_modal(&saved_task));
                    }
                }
            }
        }
    }

    Html(format!(
        "<div class=\"modal-overlay\"><div class=\"window\"><div class=\"window-pane\">Task '{}' not found</div></div></div>",
        id
    ))
}

// POST /tasks/:id/schedule-type - Re-render form with new schedule type (doesn't save)
async fn change_schedule_type(
    State(pool): State<DbPool>,
    Path(id): Path<String>,
    Form(form): Form<TaskForm>,
) -> Html<String> {
    if is_demo_id(&id) {
        let tasks = get_demo_tasks();
        let tasks_guard = tasks.lock().unwrap();

        if let Some(base_task) = tasks_guard.get(&id) {
            let temp_task = form.to_demo_task(&id, base_task);
            return Html(render_task_modal(&temp_task));
        }
    } else {
        if let Ok(task_id) = id.parse::<i64>() {
            if let Ok(Some(base_task)) = db::get_task(&pool, task_id).await {
                let temp_task = form.to_demo_task(&id, &base_task);
                return Html(render_task_modal(&temp_task));
            }
        }
    }

    Html(format!(
        "<div class=\"modal-overlay\"><div class=\"window\"><div class=\"window-pane\">Task '{}' not found</div></div></div>",
        id
    ))
}

// GET /tasks/new - Show modal for creating a new task
async fn new_task_modal() -> Html<String> {
    let new_task = create_default_task();
    Html(render_new_task_modal(&new_task))
}

// POST /tasks/new - Create a new task
async fn create_task(State(pool): State<DbPool>, Form(form): Form<TaskForm>) -> Html<String> {
    let base_task = create_default_task();
    let new_task = form.to_demo_task("", &base_task);

    // Save to database
    match db::save_task(&pool, &new_task).await {
        Ok(_) => {
            // Return empty modal container (closes the modal) and trigger list refresh
            Html(r##"<div hx-get="/tasks/list" hx-trigger="load" hx-target="#task-list" hx-swap="innerHTML"></div>"##.to_string())
        }
        Err(e) => {
            Html(format!(
                "<div class=\"modal-overlay\"><div class=\"window\"><div class=\"window-pane\">Error creating task: {}</div></div></div>",
                e
            ))
        }
    }
}

// POST /tasks/new/schedule-type - Re-render new task form with new schedule type
async fn new_task_schedule_type(Form(form): Form<TaskForm>) -> Html<String> {
    let base_task = create_default_task();
    let temp_task = form.to_demo_task("", &base_task);
    Html(render_new_task_modal(&temp_task))
}

fn create_default_task() -> DemoTask {
    DemoTask {
        id: String::new(),
        name: String::new(),
        details: String::new(),
        schedule_kind: ScheduleKind::NDays,
        n_days: default_n_days(),
        n_weeks: default_n_weeks(),
        monthwise: default_monthwise(),
        weeks_of_month: default_weeks_of_month(),
    }
}

#[derive(Clone)]
pub struct DemoTask {
    pub id: String,
    pub name: String,
    pub details: String,
    pub schedule_kind: ScheduleKind,
    pub n_days: NDays,
    pub n_weeks: NWeeks,
    pub monthwise: Monthwise,
    pub weeks_of_month: WeeksOfMonth,
}

impl DemoTask {
    /// Calculate the next due date for this task
    pub fn next_due_date(&self) -> DateTime<Utc> {
        let now = Utc::now();
        let local_now: DateTime<Local> = now.into();

        match self.schedule_kind {
            ScheduleKind::NDays => {
                // Get today at the scheduled time
                let today_at_time = local_now
                    .date_naive()
                    .and_time(self.n_days.time)
                    .and_local_timezone(Local)
                    .unwrap()
                    .with_timezone(&Utc);

                if today_at_time > now {
                    today_at_time
                } else {
                    today_at_time + Duration::days(self.n_days.days as i64)
                }
            }
            ScheduleKind::NWeeks => {
                // Find the next active day
                for days_ahead in 0..=(7 * self.n_weeks.weeks) {
                    let check_date = local_now + Duration::days(days_ahead as i64);
                    if self.n_weeks.sub_schedule.active(check_date.weekday()) {
                        let at_time = check_date
                            .date_naive()
                            .and_time(self.n_weeks.sub_schedule.time)
                            .and_local_timezone(Local)
                            .unwrap()
                            .with_timezone(&Utc);
                        if at_time > now {
                            return at_time;
                        }
                    }
                }
                now + Duration::days(7)
            }
            ScheduleKind::Monthwise => {
                let today_day = local_now.date_naive().day() as i32;

                // Check for next day in current month
                for &day in &self.monthwise.days {
                    if day > today_day {
                        if let Some(date) = local_now.with_day(day as u32) {
                            return date
                                .date_naive()
                                .and_time(self.monthwise.time)
                                .and_local_timezone(Local)
                                .unwrap()
                                .with_timezone(&Utc);
                        }
                    } else if day == today_day {
                        let at_time = local_now
                            .date_naive()
                            .and_time(self.monthwise.time)
                            .and_local_timezone(Local)
                            .unwrap()
                            .with_timezone(&Utc);
                        if at_time > now {
                            return at_time;
                        }
                    }
                }

                // Next month
                let next_month = if local_now.month() == 12 {
                    local_now
                        .with_year(local_now.year() + 1)
                        .and_then(|d| d.with_month(1))
                } else {
                    local_now.with_month(local_now.month() + 1)
                };

                if let Some(nm) = next_month {
                    if let Some(&first_day) = self.monthwise.days.iter().min() {
                        if let Some(date) = nm.with_day(first_day as u32) {
                            return date
                                .date_naive()
                                .and_time(self.monthwise.time)
                                .and_local_timezone(Local)
                                .unwrap()
                                .with_timezone(&Utc);
                        }
                    }
                }

                now + Duration::days(30)
            }
            ScheduleKind::WeeksOfMonth => {
                // Look ahead for the next matching date
                for days_ahead in 0..=60 {
                    let check_date = local_now + Duration::days(days_ahead as i64);
                    let week_num = ((check_date.day() - 1) / 7 + 1) as i32;

                    if self.weeks_of_month.sub_schedule.active(check_date.weekday())
                        && self.weeks_of_month.weeks.contains(&week_num)
                    {
                        let at_time = check_date
                            .date_naive()
                            .and_time(self.weeks_of_month.sub_schedule.time)
                            .and_local_timezone(Local)
                            .unwrap()
                            .with_timezone(&Utc);
                        if at_time > now {
                            return at_time;
                        }
                    }
                }
                now + Duration::days(30)
            }
        }
    }

    /// Format the next due date as a human-readable string
    pub fn time_as_readable_string(&self) -> String {
        let next_due = self.next_due_date();
        let local: DateTime<Local> = next_due.into();
        let now_local = Local::now();

        // Get dates without time for comparison
        let due_date = local.date_naive();
        let today = now_local.date_naive();
        let yesterday = today - Duration::days(1);
        let tomorrow = today + Duration::days(1);
        let overmorrow = today + Duration::days(2);

        let time_str = local.format("%H:%M").to_string();

        if due_date == yesterday {
            format!("Yesterday at {}", time_str)
        } else if due_date == today {
            format!("Today at {}", time_str)
        } else if due_date == tomorrow {
            format!("Tomorrow at {}", time_str)
        } else if due_date == overmorrow {
            format!("Overmorrow at {}", time_str)
        } else {
            // "{day name}, {month} {day}" at {time}
            local.format("%A, %B %-d at %H:%M").to_string()
        }
    }

    /// Check if the task is due (past its due date)
    pub fn is_due(&self) -> bool {
        self.next_due_date() <= Utc::now()
    }

    /// Check if the task is alerting (due within the next 24 hours but not yet due)
    pub fn is_alerting(&self) -> bool {
        let next_due = self.next_due_date();
        let now = Utc::now();
        let in_24_hours = now + Duration::hours(24);

        next_due > now && next_due <= in_24_hours
    }

    /// Calculate the most recent past due date for this task
    /// Used to determine if a completion happened after the task became due
    pub fn most_recent_due_date(&self) -> DateTime<Utc> {
        let now = Utc::now();
        let local_now: DateTime<Local> = now.into();

        match self.schedule_kind {
            ScheduleKind::NDays => {
                // Get today at the scheduled time
                let today_at_time = local_now
                    .date_naive()
                    .and_time(self.n_days.time)
                    .and_local_timezone(Local)
                    .unwrap()
                    .with_timezone(&Utc);

                if today_at_time > now {
                    // Most recent due was n days ago
                    today_at_time - Duration::days(self.n_days.days as i64)
                } else {
                    today_at_time
                }
            }
            ScheduleKind::NWeeks => {
                // Find the most recent past active day
                for days_back in 0..=(7 * self.n_weeks.weeks) {
                    let check_date = local_now - Duration::days(days_back as i64);
                    if self.n_weeks.sub_schedule.active(check_date.weekday()) {
                        let at_time = check_date
                            .date_naive()
                            .and_time(self.n_weeks.sub_schedule.time)
                            .and_local_timezone(Local)
                            .unwrap()
                            .with_timezone(&Utc);
                        if at_time <= now {
                            return at_time;
                        }
                    }
                }
                now - Duration::days(7)
            }
            ScheduleKind::Monthwise => {
                let today_day = local_now.date_naive().day() as i32;

                // Check for most recent day in current month
                for &day in self.monthwise.days.iter().rev() {
                    if day < today_day {
                        if let Some(date) = local_now.with_day(day as u32) {
                            return date
                                .date_naive()
                                .and_time(self.monthwise.time)
                                .and_local_timezone(Local)
                                .unwrap()
                                .with_timezone(&Utc);
                        }
                    } else if day == today_day {
                        let at_time = local_now
                            .date_naive()
                            .and_time(self.monthwise.time)
                            .and_local_timezone(Local)
                            .unwrap()
                            .with_timezone(&Utc);
                        if at_time <= now {
                            return at_time;
                        }
                    }
                }

                // Previous month
                let prev_month = if local_now.month() == 1 {
                    local_now
                        .with_year(local_now.year() - 1)
                        .and_then(|d| d.with_month(12))
                } else {
                    local_now.with_month(local_now.month() - 1)
                };

                if let Some(pm) = prev_month {
                    if let Some(&last_day) = self.monthwise.days.iter().max() {
                        if let Some(date) = pm.with_day(last_day as u32) {
                            return date
                                .date_naive()
                                .and_time(self.monthwise.time)
                                .and_local_timezone(Local)
                                .unwrap()
                                .with_timezone(&Utc);
                        }
                    }
                }

                now - Duration::days(30)
            }
            ScheduleKind::WeeksOfMonth => {
                // Look back for the most recent matching date
                for days_back in 0..=60 {
                    let check_date = local_now - Duration::days(days_back as i64);
                    let week_num = ((check_date.day() - 1) / 7 + 1) as i32;

                    if self.weeks_of_month.sub_schedule.active(check_date.weekday())
                        && self.weeks_of_month.weeks.contains(&week_num)
                    {
                        let at_time = check_date
                            .date_naive()
                            .and_time(self.weeks_of_month.sub_schedule.time)
                            .and_local_timezone(Local)
                            .unwrap()
                            .with_timezone(&Utc);
                        if at_time <= now {
                            return at_time;
                        }
                    }
                }
                now - Duration::days(30)
            }
        }
    }
}

pub fn default_n_days() -> NDays {
    NDays {
        days: 1,
        time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
    }
}

pub fn default_n_weeks() -> NWeeks {
    NWeeks {
        weeks: 1,
        sub_schedule: DaysOfWeek {
            sunday: false,
            monday: true,
            tuesday: false,
            wednesday: false,
            thursday: false,
            friday: false,
            saturday: false,
            time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        },
    }
}

pub fn default_monthwise() -> Monthwise {
    Monthwise {
        days: vec![1],
        time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
    }
}

pub fn default_weeks_of_month() -> WeeksOfMonth {
    WeeksOfMonth {
        weeks: vec![1],
        sub_schedule: DaysOfWeek {
            sunday: false,
            monday: true,
            tuesday: false,
            wednesday: false,
            thursday: false,
            friday: false,
            saturday: false,
            time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        },
    }
}

// ============================================================================
// Rendering Functions
// ============================================================================

fn render_sort_select(current_sort: &str) -> String {
    let name_selected = if current_sort == "name" { " selected" } else { "" };
    let due_selected = if current_sort == "due" { " selected" } else { "" };

    format!(
        r##"<select id="sort-select" name="sort" hx-get="/tasks/list" hx-target="#task-list" hx-swap="innerHTML" hx-trigger="change">
            <option value="name"{name_selected}>Name (A-Z)</option>
            <option value="due"{due_selected}>Next Due</option>
        </select>"##
    )
}

async fn render_task_list(pool: &DbPool, sort: &str) -> String {
    // Collect all tasks from database only (demo tasks are excluded from index)
    let mut all_tasks: Vec<DemoTask> = db::get_all_tasks(pool).await.unwrap_or_default();

    // Sort tasks
    match sort {
        "due" => {
            all_tasks.sort_by(|a, b| a.next_due_date().cmp(&b.next_due_date()));
        }
        _ => {
            all_tasks.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        }
    }

    if all_tasks.is_empty() {
        return maud! {
            div .empty-list {
                p { "No tasks yet. Create your first task!" }
            }
        }
        .render()
        .into_inner();
    }

    let items: Vec<String> = all_tasks.iter().map(render_task_list_item).collect();

    maud! {
        ul .task-list {
            (Raw::dangerously_create(&items.join("\n")))
        }
    }
    .render()
    .into_inner()
}

fn render_task_list_item(task: &DemoTask) -> String {
    let edit_url = format!("/tasks/{}/edit-modal", task.id);
    let show_url = format!("/tasks/{}", task.id);
    let next_due = task.time_as_readable_string();

    maud! {
        li .task-list-item {
            (Raw::dangerously_create(&format!(
                r##"<button class="btn" hx-get="{}" hx-target="#modal-container" hx-swap="innerHTML">Edit</button>"##,
                edit_url
            )))
            a .task-name href=(show_url) { (task.name) }
            span .task-due { (next_due) }
        }
    }
    .render()
    .into_inner()
}

fn render_task_modal(task: &DemoTask) -> String {
    let editor_html = render_task_editor_inner(task, true, false);

    maud! {
        div .modal-overlay {
            (Raw::dangerously_create(&editor_html))
        }
    }
    .render()
    .into_inner()
}

fn render_new_task_modal(task: &DemoTask) -> String {
    let editor_html = render_task_editor_inner(task, true, true);

    maud! {
        div .modal-overlay {
            (Raw::dangerously_create(&editor_html))
        }
    }
    .render()
    .into_inner()
}

pub fn render_task_editor(task: &DemoTask) -> String {
    render_task_editor_inner(task, false, false)
}

fn render_task_editor_inner(task: &DemoTask, is_modal: bool, is_new: bool) -> String {
    let schedule_label = match task.schedule_kind {
        ScheduleKind::NDays => "Every N Days",
        ScheduleKind::NWeeks => "Weekly",
        ScheduleKind::Monthwise => "Monthly (by date)",
        ScheduleKind::WeeksOfMonth => "Monthly (by weekday)",
    };

    // Use "new" as the ID suffix for new tasks
    let id_suffix = if is_new { "new".to_string() } else { task.id.clone() };

    let schedule_editor_html = match task.schedule_kind {
        ScheduleKind::NDays => render_n_days_editor(&id_suffix, &task.n_days),
        ScheduleKind::NWeeks => render_n_weeks_editor(&id_suffix, &task.n_weeks),
        ScheduleKind::Monthwise => render_monthwise_editor(&id_suffix, &task.monthwise),
        ScheduleKind::WeeksOfMonth => render_weeks_of_month_editor(&id_suffix, &task.weeks_of_month),
    };

    let is_n_days = matches!(task.schedule_kind, ScheduleKind::NDays);
    let is_n_weeks = matches!(task.schedule_kind, ScheduleKind::NWeeks);
    let is_monthwise = matches!(task.schedule_kind, ScheduleKind::Monthwise);
    let is_weeks_of_month = matches!(task.schedule_kind, ScheduleKind::WeeksOfMonth);

    let name_id = format!("task-name-{}", id_suffix);
    let details_id = format!("task-details-{}", id_suffix);
    let schedule_type_id = format!("task-schedule-type-{}", id_suffix);
    let editor_id = format!("task-editor-{}", id_suffix);

    // Use /tasks/new endpoints for new tasks
    let hx_schedule_type_post = if is_new {
        "/tasks/new/schedule-type".to_string()
    } else {
        format!("/tasks/{}/schedule-type", task.id)
    };
    let hx_save_post = if is_new {
        "/tasks/new".to_string()
    } else {
        format!("/tasks/{}", task.id)
    };
    let hx_target = if is_modal {
        "#modal-container".to_string()
    } else {
        format!("#{}", editor_id)
    };

    // For modal, cancel closes the modal; for standalone, it reloads from saved state
    let cancel_button = if is_modal {
        r##"<button class="btn" type="button" onclick="document.getElementById('modal-container').innerHTML = ''">Cancel</button>"##.to_string()
    } else {
        format!(
            r##"<button class="btn" type="button" hx-get="/tasks/{}/edit" hx-target="#{}" hx-swap="outerHTML">Cancel</button>"##,
            task.id, editor_id
        )
    };

    // Save button - for modal, close modal and reload page; for standalone, swap in place
    let save_button = if is_modal {
        format!(
            r##"<button class="btn btn-default" type="button" hx-post="{}" hx-target="{}" hx-swap="innerHTML" hx-include="closest form" hx-on::after-request="location.reload()">Save</button>"##,
            hx_save_post, hx_target
        )
    } else {
        format!(
            r##"<button class="btn btn-default" type="button" hx-post="{}" hx-target="{}" hx-swap="outerHTML" hx-include="closest form">Save</button>"##,
            hx_save_post, hx_target
        )
    };

    // Close button - for modal, clicking X closes without saving
    let close_button = if is_modal {
        r##"<button class="close" aria-label="Close" onclick="document.getElementById('modal-container').innerHTML = ''"></button>"##.to_string()
    } else {
        r#"<button class="close" aria-label="Close"></button>"#.to_string()
    };

    // Title varies based on whether this is a new task or editing
    let title = if is_new {
        "New Task".to_string()
    } else {
        format!("Edit Task: {}", task.name)
    };

    maud! {
        div .window.task-editor id=(editor_id) {
            div .title-bar {
                (Raw::dangerously_create(&close_button))
                h1 .title { (title) }
                button .hidden aria-label="Resize" disabled {}
            }
            div .separator {}

            div .window-pane {
                form {
                    div .form-group {
                        label for=(name_id) { "Name" }
                        input
                            type="text"
                            id=(name_id)
                            name="name"
                            value=(task.name);
                    }

                    div .form-group {
                        label for=(details_id) { "Details" }
                        textarea
                            id=(details_id)
                            name="details"
                        { (task.details) }
                    }

                    div .form-group {
                        label for=(schedule_type_id) { "Schedule Type" }
                        (Raw::dangerously_create(&render_schedule_type_select(
                            &schedule_type_id,
                            &hx_schedule_type_post,
                            &hx_target,
                            is_n_days,
                            is_n_weeks,
                            is_monthwise,
                            is_weeks_of_month,
                        )))
                    }

                    div .schedule-editor {
                        h4 { (schedule_label) " Settings" }
                        (Raw::dangerously_create(&schedule_editor_html))
                    }

                    div .form-group style="margin-top: 16px; text-align: right;" {
                        (Raw::dangerously_create(&cancel_button))
                        " "
                        (Raw::dangerously_create(&save_button))
                    }
                }
            }
        }
    }
    .render()
    .into_inner()
}

fn render_schedule_type_select(
    id: &str,
    hx_post: &str,
    hx_target: &str,
    is_n_days: bool,
    is_n_weeks: bool,
    is_monthwise: bool,
    is_weeks_of_month: bool,
) -> String {
    let n_days_selected = if is_n_days { " selected" } else { "" };
    let n_weeks_selected = if is_n_weeks { " selected" } else { "" };
    let monthwise_selected = if is_monthwise { " selected" } else { "" };
    let weeks_of_month_selected = if is_weeks_of_month { " selected" } else { "" };

    format!(
        r#"<select id="{id}" name="schedule_type" hx-post="{hx_post}" hx-target="{hx_target}" hx-swap="innerHTML" hx-trigger="change" hx-include="closest form">
            <option value="n_days"{n_days_selected}>Every N Days</option>
            <option value="n_weeks"{n_weeks_selected}>Weekly</option>
            <option value="monthwise"{monthwise_selected}>Monthly (by date)</option>
            <option value="weeks_of_month"{weeks_of_month_selected}>Monthly (by weekday)</option>
        </select>"#
    )
}

fn render_n_days_editor(task_id: &str, n_days: &NDays) -> String {
    let count_id = format!("n-days-count-{}", task_id);
    let time_id = format!("n-days-time-{}", task_id);
    let time_value = n_days.time.format("%H:%M").to_string();

    maud! {
        div .form-group {
            div .inline-field {
                label for=(count_id) { "Every" }
                input
                    type="number"
                    id=(count_id)
                    name="n_days_count"
                    min="1"
                    value=(n_days.days);
                span { "day(s)" }
            }
        }
        div .form-group {
            div .inline-field {
                label for=(time_id) { "At" }
                input
                    type="time"
                    id=(time_id)
                    name="n_days_time"
                    value=(time_value);
            }
        }
    }
    .render()
    .into_inner()
}

fn render_n_weeks_editor(task_id: &str, n_weeks: &NWeeks) -> String {
    let count_id = format!("n-weeks-count-{}", task_id);
    let time_id = format!("n-weeks-time-{}", task_id);
    let time_value = n_weeks.sub_schedule.time.format("%H:%M").to_string();

    let sun_id = format!("dow-sun-{}", task_id);
    let mon_id = format!("dow-mon-{}", task_id);
    let tue_id = format!("dow-tue-{}", task_id);
    let wed_id = format!("dow-wed-{}", task_id);
    let thu_id = format!("dow-thu-{}", task_id);
    let fri_id = format!("dow-fri-{}", task_id);
    let sat_id = format!("dow-sat-{}", task_id);

    maud! {
        div .form-group {
            div .inline-field {
                label for=(count_id) { "Every" }
                input
                    type="number"
                    id=(count_id)
                    name="n_weeks_count"
                    min="1"
                    value=(n_weeks.weeks);
                span { "week(s)" }
            }
        }
        div .form-group {
            label { "On days:" }
            div .days-grid {
                div .field-row {
                    @if n_weeks.sub_schedule.sunday {
                        input type="checkbox" id=(sun_id) name="dow_sun" checked;
                    } @else {
                        input type="checkbox" id=(sun_id) name="dow_sun";
                    }
                    label for=(sun_id) { "Sun" }
                }
                div .field-row {
                    @if n_weeks.sub_schedule.monday {
                        input type="checkbox" id=(mon_id) name="dow_mon" checked;
                    } @else {
                        input type="checkbox" id=(mon_id) name="dow_mon";
                    }
                    label for=(mon_id) { "Mon" }
                }
                div .field-row {
                    @if n_weeks.sub_schedule.tuesday {
                        input type="checkbox" id=(tue_id) name="dow_tue" checked;
                    } @else {
                        input type="checkbox" id=(tue_id) name="dow_tue";
                    }
                    label for=(tue_id) { "Tue" }
                }
                div .field-row {
                    @if n_weeks.sub_schedule.wednesday {
                        input type="checkbox" id=(wed_id) name="dow_wed" checked;
                    } @else {
                        input type="checkbox" id=(wed_id) name="dow_wed";
                    }
                    label for=(wed_id) { "Wed" }
                }
                div .field-row {
                    @if n_weeks.sub_schedule.thursday {
                        input type="checkbox" id=(thu_id) name="dow_thu" checked;
                    } @else {
                        input type="checkbox" id=(thu_id) name="dow_thu";
                    }
                    label for=(thu_id) { "Thu" }
                }
                div .field-row {
                    @if n_weeks.sub_schedule.friday {
                        input type="checkbox" id=(fri_id) name="dow_fri" checked;
                    } @else {
                        input type="checkbox" id=(fri_id) name="dow_fri";
                    }
                    label for=(fri_id) { "Fri" }
                }
                div .field-row {
                    @if n_weeks.sub_schedule.saturday {
                        input type="checkbox" id=(sat_id) name="dow_sat" checked;
                    } @else {
                        input type="checkbox" id=(sat_id) name="dow_sat";
                    }
                    label for=(sat_id) { "Sat" }
                }
            }
        }
        div .form-group {
            div .inline-field {
                label for=(time_id) { "At" }
                input
                    type="time"
                    id=(time_id)
                    name="n_weeks_time"
                    value=(time_value);
            }
        }
    }
    .render()
    .into_inner()
}

fn render_monthwise_editor(task_id: &str, monthwise: &Monthwise) -> String {
    let days_id = format!("monthwise-days-{}", task_id);
    let time_id = format!("monthwise-time-{}", task_id);
    let time_value = monthwise.time.format("%H:%M").to_string();

    let days_str = monthwise
        .days
        .iter()
        .map(|d| d.to_string())
        .collect::<Vec<_>>()
        .join(", ");

    maud! {
        div .form-group {
            label for=(days_id) { "On day(s) of month:" }
            input
                type="text"
                id=(days_id)
                name="monthwise_days"
                placeholder="e.g. 1, 15"
                value=(days_str);
            small style="display: block; color: #666; margin-top: 4px;" {
                "Comma-separated list of days (1-31)"
            }
        }
        div .form-group {
            div .inline-field {
                label for=(time_id) { "At" }
                input
                    type="time"
                    id=(time_id)
                    name="monthwise_time"
                    value=(time_value);
            }
        }
    }
    .render()
    .into_inner()
}

fn render_weeks_of_month_editor(task_id: &str, weeks_of_month: &WeeksOfMonth) -> String {
    let time_id = format!("wom-time-{}", task_id);
    let time_value = weeks_of_month.sub_schedule.time.format("%H:%M").to_string();

    let week_labels = ["1st", "2nd", "3rd", "4th", "5th"];

    let weeks_html: String = (1..=5i32)
        .map(|week| {
            let week_id = format!("wom-week-{}-{}", task_id, week);
            let week_name = format!("wom_week_{}", week);
            let is_checked = weeks_of_month.weeks.contains(&week);
            let label = week_labels[(week - 1) as usize];

            if is_checked {
                format!(
                    r#"<div class="field-row"><input type="checkbox" id="{}" name="{}" checked><label for="{}">{}</label></div>"#,
                    week_id, week_name, week_id, label
                )
            } else {
                format!(
                    r#"<div class="field-row"><input type="checkbox" id="{}" name="{}"><label for="{}">{}</label></div>"#,
                    week_id, week_name, week_id, label
                )
            }
        })
        .collect();

    let sun_id = format!("wom-dow-sun-{}", task_id);
    let mon_id = format!("wom-dow-mon-{}", task_id);
    let tue_id = format!("wom-dow-tue-{}", task_id);
    let wed_id = format!("wom-dow-wed-{}", task_id);
    let thu_id = format!("wom-dow-thu-{}", task_id);
    let fri_id = format!("wom-dow-fri-{}", task_id);
    let sat_id = format!("wom-dow-sat-{}", task_id);

    maud! {
        div .form-group {
            label { "Week(s) of month:" }
            div .weeks-checkboxes {
                (Raw::dangerously_create(&weeks_html))
            }
        }
        div .form-group {
            label { "On days:" }
            div .days-grid {
                div .field-row {
                    @if weeks_of_month.sub_schedule.sunday {
                        input type="checkbox" id=(sun_id) name="wom_dow_sun" checked;
                    } @else {
                        input type="checkbox" id=(sun_id) name="wom_dow_sun";
                    }
                    label for=(sun_id) { "Sun" }
                }
                div .field-row {
                    @if weeks_of_month.sub_schedule.monday {
                        input type="checkbox" id=(mon_id) name="wom_dow_mon" checked;
                    } @else {
                        input type="checkbox" id=(mon_id) name="wom_dow_mon";
                    }
                    label for=(mon_id) { "Mon" }
                }
                div .field-row {
                    @if weeks_of_month.sub_schedule.tuesday {
                        input type="checkbox" id=(tue_id) name="wom_dow_tue" checked;
                    } @else {
                        input type="checkbox" id=(tue_id) name="wom_dow_tue";
                    }
                    label for=(tue_id) { "Tue" }
                }
                div .field-row {
                    @if weeks_of_month.sub_schedule.wednesday {
                        input type="checkbox" id=(wed_id) name="wom_dow_wed" checked;
                    } @else {
                        input type="checkbox" id=(wed_id) name="wom_dow_wed";
                    }
                    label for=(wed_id) { "Wed" }
                }
                div .field-row {
                    @if weeks_of_month.sub_schedule.thursday {
                        input type="checkbox" id=(thu_id) name="wom_dow_thu" checked;
                    } @else {
                        input type="checkbox" id=(thu_id) name="wom_dow_thu";
                    }
                    label for=(thu_id) { "Thu" }
                }
                div .field-row {
                    @if weeks_of_month.sub_schedule.friday {
                        input type="checkbox" id=(fri_id) name="wom_dow_fri" checked;
                    } @else {
                        input type="checkbox" id=(fri_id) name="wom_dow_fri";
                    }
                    label for=(fri_id) { "Fri" }
                }
                div .field-row {
                    @if weeks_of_month.sub_schedule.saturday {
                        input type="checkbox" id=(sat_id) name="wom_dow_sat" checked;
                    } @else {
                        input type="checkbox" id=(sat_id) name="wom_dow_sat";
                    }
                    label for=(sat_id) { "Sat" }
                }
            }
        }
        div .form-group {
            div .inline-field {
                label for=(time_id) { "At" }
                input
                    type="time"
                    id=(time_id)
                    name="wom_time"
                    value=(time_value);
            }
        }
    }
    .render()
    .into_inner()
}
