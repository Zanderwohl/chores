-- Initial schema for chores application

-- Migration tracking table
CREATE TABLE IF NOT EXISTS migrations (
    timestamp TEXT PRIMARY KEY,
    applied INTEGER NOT NULL DEFAULT 0,
    applied_at TEXT
);

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
);

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
);

CREATE TABLE IF NOT EXISTS completions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL,
    completed_at TEXT NOT NULL
);
