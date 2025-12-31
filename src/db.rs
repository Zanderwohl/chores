use anyhow::Result;
use chrono::{DateTime, NaiveTime, Utc};
use sqlx::{sqlite::SqlitePool, FromRow, Row};

use crate::schedule::{CertainMonths, DaysOfWeek, Monthwise, NDays, NWeeks, Once, ScheduleKind, WeeksOfMonth};
use crate::tasks::DemoTask;

pub type DbPool = SqlitePool;

pub async fn init_db(database_url: &str) -> Result<DbPool> {
    let pool = SqlitePool::connect(database_url).await?;
    create_tables(&pool).await?;
    Ok(pool)
}

async fn create_tables(pool: &DbPool) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS schedules (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            -- NDays fields
            ndays_days INTEGER,
            ndays_time TEXT,
            -- NWeeks fields
            nweeks_weeks INTEGER,
            nweeks_sunday INTEGER,
            nweeks_monday INTEGER,
            nweeks_tuesday INTEGER,
            nweeks_wednesday INTEGER,
            nweeks_thursday INTEGER,
            nweeks_friday INTEGER,
            nweeks_saturday INTEGER,
            nweeks_time TEXT,
            -- Monthwise fields
            monthwise_days TEXT,
            monthwise_time TEXT,
            -- WeeksOfMonth fields
            weeks_of_month_weeks TEXT,
            weeks_of_month_sunday INTEGER,
            weeks_of_month_monday INTEGER,
            weeks_of_month_tuesday INTEGER,
            weeks_of_month_wednesday INTEGER,
            weeks_of_month_thursday INTEGER,
            weeks_of_month_friday INTEGER,
            weeks_of_month_saturday INTEGER,
            weeks_of_month_time TEXT,
            -- CertainMonths fields
            certain_months_months TEXT,
            certain_months_days TEXT,
            certain_months_time TEXT,
            -- Once fields
            once_datetime TEXT
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS tasks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            details TEXT,
            schedule_id INTEGER NOT NULL,
            alerting_time INTEGER,
            completeable INTEGER NOT NULL DEFAULT 1,
            created_at TEXT,
            deleted_at TEXT,
            FOREIGN KEY (schedule_id) REFERENCES schedules(id)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS completions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id TEXT NOT NULL,
            completed_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}

// Add a completion record for a task
pub async fn add_completion(pool: &DbPool, task_id: &str) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("INSERT INTO completions (task_id, completed_at) VALUES (?, ?)")
        .bind(task_id)
        .bind(now)
        .execute(pool)
        .await?;
    Ok(())
}

// Get the latest completion for a task
pub async fn get_latest_completion(pool: &DbPool, task_id: &str) -> Result<Option<chrono::DateTime<chrono::Utc>>> {
    let result: Option<(String,)> = sqlx::query_as(
        "SELECT completed_at FROM completions WHERE task_id = ? ORDER BY completed_at DESC LIMIT 1"
    )
        .bind(task_id)
        .fetch_optional(pool)
        .await?;

    Ok(result.and_then(|(s,)| chrono::DateTime::parse_from_rfc3339(&s).ok().map(|dt| dt.with_timezone(&chrono::Utc))))
}

// Completion record with ID for display
pub struct CompletionRecord {
    pub id: i64,
    pub completed_at: chrono::DateTime<chrono::Utc>,
}

// Get all completions for a task (most recent first)
pub async fn get_all_completions(pool: &DbPool, task_id: &str) -> Result<Vec<CompletionRecord>> {
    let results: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id, completed_at FROM completions WHERE task_id = ? ORDER BY completed_at DESC"
    )
        .bind(task_id)
        .fetch_all(pool)
        .await?;

    Ok(results
        .into_iter()
        .filter_map(|(id, s)| {
            chrono::DateTime::parse_from_rfc3339(&s)
                .ok()
                .map(|dt| CompletionRecord {
                    id,
                    completed_at: dt.with_timezone(&chrono::Utc),
                })
        })
        .collect())
}

// Delete a completion by ID
pub async fn delete_completion(pool: &DbPool, completion_id: i64) -> Result<()> {
    sqlx::query("DELETE FROM completions WHERE id = ?")
        .bind(completion_id)
        .execute(pool)
        .await?;
    Ok(())
}

#[derive(Debug, FromRow)]
pub struct DbSchedule {
    pub id: i64,
    pub kind: String,
    // NDays
    pub ndays_days: Option<i32>,
    pub ndays_time: Option<String>,
    // NWeeks
    pub nweeks_weeks: Option<i32>,
    pub nweeks_sunday: Option<i32>,
    pub nweeks_monday: Option<i32>,
    pub nweeks_tuesday: Option<i32>,
    pub nweeks_wednesday: Option<i32>,
    pub nweeks_thursday: Option<i32>,
    pub nweeks_friday: Option<i32>,
    pub nweeks_saturday: Option<i32>,
    pub nweeks_time: Option<String>,
    // Monthwise
    pub monthwise_days: Option<String>,
    pub monthwise_time: Option<String>,
    // WeeksOfMonth
    pub weeks_of_month_weeks: Option<String>,
    pub weeks_of_month_sunday: Option<i32>,
    pub weeks_of_month_monday: Option<i32>,
    pub weeks_of_month_tuesday: Option<i32>,
    pub weeks_of_month_wednesday: Option<i32>,
    pub weeks_of_month_thursday: Option<i32>,
    pub weeks_of_month_friday: Option<i32>,
    pub weeks_of_month_saturday: Option<i32>,
    pub weeks_of_month_time: Option<String>,
    // CertainMonths
    pub certain_months_months: Option<String>,
    pub certain_months_days: Option<String>,
    pub certain_months_time: Option<String>,
    // Once
    pub once_datetime: Option<String>,
}

#[derive(Debug, FromRow)]
pub struct DbTask {
    pub id: i64,
    pub name: String,
    pub details: Option<String>,
    pub schedule_id: i64,
    pub alerting_time: Option<i64>,
    pub completeable: Option<i32>,
    pub created_at: Option<String>,
    pub deleted_at: Option<String>,
}

#[derive(Debug, FromRow)]
pub struct DbCompletion {
    pub id: i64,
    pub task_id: i64,
    pub completed_at: String,
}

// Helper to parse time from string
fn parse_time(s: &Option<String>) -> NaiveTime {
    s.as_ref()
        .and_then(|t| NaiveTime::parse_from_str(t, "%H:%M").ok())
        .unwrap_or_else(|| NaiveTime::from_hms_opt(9, 0, 0).unwrap())
}

// Helper to parse comma-separated integers
fn parse_int_list(s: &Option<String>) -> Vec<i32> {
    s.as_ref()
        .map(|s| {
            s.split(',')
                .filter_map(|part| part.trim().parse::<i32>().ok())
                .collect()
        })
        .unwrap_or_default()
}

impl DbSchedule {
    pub fn to_schedule_parts(&self) -> (ScheduleKind, NDays, NWeeks, Monthwise, WeeksOfMonth, CertainMonths, Once) {
        let kind = match self.kind.as_str() {
            "n_days" => ScheduleKind::NDays,
            "n_weeks" => ScheduleKind::NWeeks,
            "monthwise" => ScheduleKind::Monthwise,
            "weeks_of_month" => ScheduleKind::WeeksOfMonth,
            "certain_months" => ScheduleKind::CertainMonths,
            "once" => ScheduleKind::Once,
            _ => ScheduleKind::NDays,
        };

        let n_days = NDays {
            days: self.ndays_days.unwrap_or(1),
            time: parse_time(&self.ndays_time),
        };

        let n_weeks = NWeeks {
            weeks: self.nweeks_weeks.unwrap_or(1),
            sub_schedule: DaysOfWeek {
                sunday: self.nweeks_sunday.unwrap_or(0) != 0,
                monday: self.nweeks_monday.unwrap_or(0) != 0,
                tuesday: self.nweeks_tuesday.unwrap_or(0) != 0,
                wednesday: self.nweeks_wednesday.unwrap_or(0) != 0,
                thursday: self.nweeks_thursday.unwrap_or(0) != 0,
                friday: self.nweeks_friday.unwrap_or(0) != 0,
                saturday: self.nweeks_saturday.unwrap_or(0) != 0,
                time: parse_time(&self.nweeks_time),
            },
        };

        let monthwise = Monthwise {
            days: parse_int_list(&self.monthwise_days),
            time: parse_time(&self.monthwise_time),
        };

        let weeks_of_month = WeeksOfMonth {
            weeks: parse_int_list(&self.weeks_of_month_weeks),
            sub_schedule: DaysOfWeek {
                sunday: self.weeks_of_month_sunday.unwrap_or(0) != 0,
                monday: self.weeks_of_month_monday.unwrap_or(0) != 0,
                tuesday: self.weeks_of_month_tuesday.unwrap_or(0) != 0,
                wednesday: self.weeks_of_month_wednesday.unwrap_or(0) != 0,
                thursday: self.weeks_of_month_thursday.unwrap_or(0) != 0,
                friday: self.weeks_of_month_friday.unwrap_or(0) != 0,
                saturday: self.weeks_of_month_saturday.unwrap_or(0) != 0,
                time: parse_time(&self.weeks_of_month_time),
            },
        };

        let certain_months = CertainMonths {
            months: parse_int_list(&self.certain_months_months),
            days: parse_int_list(&self.certain_months_days),
            time: parse_time(&self.certain_months_time),
        };

        let once = Once {
            datetime: self.once_datetime.as_ref()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(Utc::now),
        };

        (kind, n_days, n_weeks, monthwise, weeks_of_month, certain_months, once)
    }
}

// Get a task by ID from the database
pub async fn get_task(pool: &DbPool, task_id: i64) -> Result<Option<DemoTask>> {
    let task: Option<DbTask> = sqlx::query_as("SELECT * FROM tasks WHERE id = ?")
        .bind(task_id)
        .fetch_optional(pool)
        .await?;

    let Some(task) = task else {
        return Ok(None);
    };

    let schedule: DbSchedule = sqlx::query_as("SELECT * FROM schedules WHERE id = ?")
        .bind(task.schedule_id)
        .fetch_one(pool)
        .await?;

    let (schedule_kind, n_days, n_weeks, monthwise, weeks_of_month, certain_months, once) = schedule.to_schedule_parts();

    let created_at = task.created_at.as_ref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));
    let deleted_at = task.deleted_at.as_ref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));

    Ok(Some(DemoTask {
        id: task_id.to_string(),
        name: task.name,
        details: task.details.unwrap_or_default(),
        schedule_kind,
        n_days,
        n_weeks,
        monthwise,
        weeks_of_month,
        certain_months,
        once,
        alerting_time: task.alerting_time.unwrap_or(1440), // Default 24 hours
        completeable: task.completeable.unwrap_or(1) != 0,
        created_at,
        deleted_at,
    }))
}

// Get all tasks from the database
pub async fn get_all_tasks(pool: &DbPool) -> Result<Vec<DemoTask>> {
    let tasks: Vec<DbTask> = sqlx::query_as("SELECT * FROM tasks")
        .fetch_all(pool)
        .await?;

    let mut result = Vec::new();

    for task in tasks {
        let schedule: DbSchedule = sqlx::query_as("SELECT * FROM schedules WHERE id = ?")
            .bind(task.schedule_id)
            .fetch_one(pool)
            .await?;

        let (schedule_kind, n_days, n_weeks, monthwise, weeks_of_month, certain_months, once) =
            schedule.to_schedule_parts();

        let created_at = task.created_at.as_ref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));
        let deleted_at = task.deleted_at.as_ref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        result.push(DemoTask {
            id: task.id.to_string(),
            name: task.name,
            details: task.details.unwrap_or_default(),
            schedule_kind,
            n_days,
            n_weeks,
            monthwise,
            weeks_of_month,
            certain_months,
            once,
            alerting_time: task.alerting_time.unwrap_or(1440), // Default 24 hours
            completeable: task.completeable.unwrap_or(1) != 0,
            created_at,
            deleted_at,
        });
    }

    Ok(result)
}

// Get total count of tasks for pagination
pub async fn get_task_count(pool: &DbPool) -> Result<i64> {
    let result: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM tasks")
        .fetch_one(pool)
        .await?;
    Ok(result.0)
}

// Get paginated tasks from the database (sorted by specified column)
pub async fn get_tasks_paginated(
    pool: &DbPool,
    sort: &str,
    offset: i64,
    limit: i64,
) -> Result<Vec<DemoTask>> {
    // Build the ORDER BY clause based on sort parameter
    let order_by = match sort {
        "due" => "id", // We'll sort by next_due in Rust since it's calculated
        _ => "name COLLATE NOCASE",
    };

    let query = format!("SELECT * FROM tasks ORDER BY {} LIMIT ? OFFSET ?", order_by);
    let tasks: Vec<DbTask> = sqlx::query_as(&query)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;

    let mut result = Vec::new();

    for task in tasks {
        let schedule: DbSchedule = sqlx::query_as("SELECT * FROM schedules WHERE id = ?")
            .bind(task.schedule_id)
            .fetch_one(pool)
            .await?;

        let (schedule_kind, n_days, n_weeks, monthwise, weeks_of_month, certain_months, once) =
            schedule.to_schedule_parts();

        let created_at = task.created_at.as_ref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));
        let deleted_at = task.deleted_at.as_ref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        result.push(DemoTask {
            id: task.id.to_string(),
            name: task.name,
            details: task.details.unwrap_or_default(),
            schedule_kind,
            n_days,
            n_weeks,
            monthwise,
            weeks_of_month,
            certain_months,
            once,
            alerting_time: task.alerting_time.unwrap_or(1440), // Default 24 hours
            completeable: task.completeable.unwrap_or(1) != 0,
            created_at,
            deleted_at,
        });
    }

    Ok(result)
}

// Save (insert or update) a task to the database
pub async fn save_task(pool: &DbPool, task: &DemoTask) -> Result<i64> {
    let task_id: Option<i64> = task.id.parse().ok();

    let kind_str = match task.schedule_kind {
        ScheduleKind::NDays => "n_days",
        ScheduleKind::NWeeks => "n_weeks",
        ScheduleKind::Monthwise => "monthwise",
        ScheduleKind::WeeksOfMonth => "weeks_of_month",
        ScheduleKind::CertainMonths => "certain_months",
        ScheduleKind::Once => "once",
    };

    let ndays_time = task.n_days.time.format("%H:%M").to_string();
    let nweeks_time = task.n_weeks.sub_schedule.time.format("%H:%M").to_string();
    let monthwise_days = task
        .monthwise
        .days
        .iter()
        .map(|d| d.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let monthwise_time = task.monthwise.time.format("%H:%M").to_string();
    let wom_weeks = task
        .weeks_of_month
        .weeks
        .iter()
        .map(|w| w.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let wom_time = task
        .weeks_of_month
        .sub_schedule
        .time
        .format("%H:%M")
        .to_string();
    let cm_months = task
        .certain_months
        .months
        .iter()
        .map(|m| m.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let cm_days = task
        .certain_months
        .days
        .iter()
        .map(|d| d.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let cm_time = task.certain_months.time.format("%H:%M").to_string();
    let once_datetime = task.once.datetime.to_rfc3339();

    // Check if task exists
    if let Some(id) = task_id {
        let existing: Option<DbTask> = sqlx::query_as("SELECT * FROM tasks WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?;

        if let Some(existing) = existing {
            // Update existing schedule
            sqlx::query(
                r#"
                UPDATE schedules SET
                    kind = ?,
                    ndays_days = ?,
                    ndays_time = ?,
                    nweeks_weeks = ?,
                    nweeks_sunday = ?,
                    nweeks_monday = ?,
                    nweeks_tuesday = ?,
                    nweeks_wednesday = ?,
                    nweeks_thursday = ?,
                    nweeks_friday = ?,
                    nweeks_saturday = ?,
                    nweeks_time = ?,
                    monthwise_days = ?,
                    monthwise_time = ?,
                    weeks_of_month_weeks = ?,
                    weeks_of_month_sunday = ?,
                    weeks_of_month_monday = ?,
                    weeks_of_month_tuesday = ?,
                    weeks_of_month_wednesday = ?,
                    weeks_of_month_thursday = ?,
                    weeks_of_month_friday = ?,
                    weeks_of_month_saturday = ?,
                    weeks_of_month_time = ?,
                    certain_months_months = ?,
                    certain_months_days = ?,
                    certain_months_time = ?,
                    once_datetime = ?
                WHERE id = ?
                "#,
            )
            .bind(kind_str)
            .bind(task.n_days.days)
            .bind(&ndays_time)
            .bind(task.n_weeks.weeks)
            .bind(task.n_weeks.sub_schedule.sunday as i32)
            .bind(task.n_weeks.sub_schedule.monday as i32)
            .bind(task.n_weeks.sub_schedule.tuesday as i32)
            .bind(task.n_weeks.sub_schedule.wednesday as i32)
            .bind(task.n_weeks.sub_schedule.thursday as i32)
            .bind(task.n_weeks.sub_schedule.friday as i32)
            .bind(task.n_weeks.sub_schedule.saturday as i32)
            .bind(&nweeks_time)
            .bind(&monthwise_days)
            .bind(&monthwise_time)
            .bind(&wom_weeks)
            .bind(task.weeks_of_month.sub_schedule.sunday as i32)
            .bind(task.weeks_of_month.sub_schedule.monday as i32)
            .bind(task.weeks_of_month.sub_schedule.tuesday as i32)
            .bind(task.weeks_of_month.sub_schedule.wednesday as i32)
            .bind(task.weeks_of_month.sub_schedule.thursday as i32)
            .bind(task.weeks_of_month.sub_schedule.friday as i32)
            .bind(task.weeks_of_month.sub_schedule.saturday as i32)
            .bind(&wom_time)
            .bind(&cm_months)
            .bind(&cm_days)
            .bind(&cm_time)
            .bind(&once_datetime)
            .bind(existing.schedule_id)
            .execute(pool)
            .await?;

            // Update existing task
            let created_at_str = task.created_at.map(|dt| dt.to_rfc3339());
            let deleted_at_str = task.deleted_at.map(|dt| dt.to_rfc3339());
            sqlx::query("UPDATE tasks SET name = ?, details = ?, alerting_time = ?, completeable = ?, created_at = ?, deleted_at = ? WHERE id = ?")
                .bind(&task.name)
                .bind(&task.details)
                .bind(task.alerting_time)
                .bind(task.completeable as i32)
                .bind(&created_at_str)
                .bind(&deleted_at_str)
                .bind(id)
                .execute(pool)
                .await?;

            return Ok(id);
        }
    }

    // Insert new schedule
    let schedule_result = sqlx::query(
        r#"
        INSERT INTO schedules (
            kind,
            ndays_days, ndays_time,
            nweeks_weeks, nweeks_sunday, nweeks_monday, nweeks_tuesday, nweeks_wednesday,
            nweeks_thursday, nweeks_friday, nweeks_saturday, nweeks_time,
            monthwise_days, monthwise_time,
            weeks_of_month_weeks, weeks_of_month_sunday, weeks_of_month_monday,
            weeks_of_month_tuesday, weeks_of_month_wednesday, weeks_of_month_thursday,
            weeks_of_month_friday, weeks_of_month_saturday, weeks_of_month_time,
            certain_months_months, certain_months_days, certain_months_time,
            once_datetime
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(kind_str)
    .bind(task.n_days.days)
    .bind(&ndays_time)
    .bind(task.n_weeks.weeks)
    .bind(task.n_weeks.sub_schedule.sunday as i32)
    .bind(task.n_weeks.sub_schedule.monday as i32)
    .bind(task.n_weeks.sub_schedule.tuesday as i32)
    .bind(task.n_weeks.sub_schedule.wednesday as i32)
    .bind(task.n_weeks.sub_schedule.thursday as i32)
    .bind(task.n_weeks.sub_schedule.friday as i32)
    .bind(task.n_weeks.sub_schedule.saturday as i32)
    .bind(&nweeks_time)
    .bind(&monthwise_days)
    .bind(&monthwise_time)
    .bind(&wom_weeks)
    .bind(task.weeks_of_month.sub_schedule.sunday as i32)
    .bind(task.weeks_of_month.sub_schedule.monday as i32)
    .bind(task.weeks_of_month.sub_schedule.tuesday as i32)
    .bind(task.weeks_of_month.sub_schedule.wednesday as i32)
    .bind(task.weeks_of_month.sub_schedule.thursday as i32)
    .bind(task.weeks_of_month.sub_schedule.friday as i32)
    .bind(task.weeks_of_month.sub_schedule.saturday as i32)
    .bind(&wom_time)
    .bind(&cm_months)
    .bind(&cm_days)
    .bind(&cm_time)
    .bind(&once_datetime)
    .execute(pool)
    .await?;

    let schedule_id = schedule_result.last_insert_rowid();

    // Insert new task
    let created_at_str = task.created_at.map(|dt| dt.to_rfc3339());
    let deleted_at_str = task.deleted_at.map(|dt| dt.to_rfc3339());
    let task_result = sqlx::query(
        "INSERT INTO tasks (name, details, schedule_id, alerting_time, completeable, created_at, deleted_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&task.name)
    .bind(&task.details)
    .bind(schedule_id)
    .bind(task.alerting_time)
    .bind(task.completeable as i32)
    .bind(&created_at_str)
    .bind(&deleted_at_str)
    .execute(pool)
    .await?;

    Ok(task_result.last_insert_rowid())
}

// Set or clear the deleted_at timestamp for a task
pub async fn set_task_deleted_at(pool: &DbPool, task_id: i64, deleted_at: Option<DateTime<Utc>>) -> Result<()> {
    let deleted_at_str = deleted_at.map(|dt| dt.to_rfc3339());
    
    sqlx::query("UPDATE tasks SET deleted_at = ? WHERE id = ?")
        .bind(&deleted_at_str)
        .bind(task_id)
        .execute(pool)
        .await?;
    
    Ok(())
}
