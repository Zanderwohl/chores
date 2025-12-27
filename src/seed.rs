//! Seed binary for populating the chores database with initial tasks.
//! 
//! Usage: cargo run --bin seed
//! 
//! Reads from seed.toml in the project root and inserts tasks into the database.

mod db;
mod schedule;
mod task;
mod tasks;

use anyhow::Result;
use chrono::NaiveTime;
use serde::Deserialize;
use std::fs;

use crate::schedule::{DaysOfWeek, Monthwise, NDays, NWeeks, ScheduleKind, WeeksOfMonth};
use crate::tasks::DemoTask;

#[derive(Debug, Deserialize)]
struct SeedData {
    tasks: Vec<SeedTask>,
}

#[derive(Debug, Deserialize)]
struct SeedTask {
    name: String,
    #[serde(default)]
    details: String,
    schedule_type: String,
    
    // NDays fields
    #[serde(default)]
    n_days: Option<i32>,
    
    // NWeeks fields
    #[serde(default)]
    n_weeks: Option<i32>,
    
    // Common fields
    #[serde(default)]
    time: Option<String>,
    #[serde(default)]
    days: Option<Vec<String>>,
    
    // Monthwise fields
    #[serde(default)]
    days_of_month: Option<Vec<i32>>,
    
    // WeeksOfMonth fields
    #[serde(default)]
    weeks: Option<Vec<i32>>,
}

impl SeedTask {
    fn to_demo_task(&self) -> DemoTask {
        let time = self.time.as_ref()
            .and_then(|t| NaiveTime::parse_from_str(t, "%H:%M").ok())
            .unwrap_or_else(|| NaiveTime::from_hms_opt(9, 0, 0).unwrap());
        
        let schedule_kind = match self.schedule_type.as_str() {
            "n_days" => ScheduleKind::NDays,
            "n_weeks" => ScheduleKind::NWeeks,
            "monthwise" => ScheduleKind::Monthwise,
            "weeks_of_month" => ScheduleKind::WeeksOfMonth,
            _ => ScheduleKind::NDays,
        };
        
        let n_days = NDays {
            days: self.n_days.unwrap_or(1),
            time,
        };
        
        let days_of_week = self.parse_days_of_week(time);
        
        let n_weeks = NWeeks {
            weeks: self.n_weeks.unwrap_or(1),
            sub_schedule: days_of_week.clone(),
        };
        
        let monthwise = Monthwise {
            days: self.days_of_month.clone().unwrap_or_else(|| vec![1]),
            time,
        };
        
        let weeks_of_month = WeeksOfMonth {
            weeks: self.weeks.clone().unwrap_or_else(|| vec![1]),
            sub_schedule: days_of_week,
        };
        
        DemoTask {
            id: String::new(), // Will be assigned by database
            name: self.name.clone(),
            details: self.details.clone(),
            schedule_kind,
            n_days,
            n_weeks,
            monthwise,
            weeks_of_month,
        }
    }
    
    fn parse_days_of_week(&self, time: NaiveTime) -> DaysOfWeek {
        let days = self.days.as_ref();
        
        let contains = |day: &str| -> bool {
            days.map_or(false, |d| d.iter().any(|s| s.to_lowercase() == day))
        };
        
        DaysOfWeek {
            sunday: contains("sunday"),
            monday: contains("monday"),
            tuesday: contains("tuesday"),
            wednesday: contains("wednesday"),
            thursday: contains("thursday"),
            friday: contains("friday"),
            saturday: contains("saturday"),
            time,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("🌱 Seeding database...");
    
    // Load environment
    let _ = dotenvy::EnvLoader::new().load();
    
    // Connect to database
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite:chores.db?mode=rwc".to_string());
    let pool = db::init_db(&database_url).await?;
    println!("📦 Connected to database: {}", database_url);
    
    // Read seed file
    let seed_content = fs::read_to_string("seed.toml")?;
    let seed_data: SeedData = toml::from_str(&seed_content)?;
    
    println!("📋 Found {} tasks to seed", seed_data.tasks.len());
    
    // Insert each task
    for seed_task in seed_data.tasks {
        let task = seed_task.to_demo_task();
        match db::save_task(&pool, &task).await {
            Ok(id) => println!("  ✓ Created task: {} (id: {})", task.name, id),
            Err(e) => println!("  ✗ Failed to create task {}: {}", task.name, e),
        }
    }
    
    println!("✅ Seeding complete!");
    
    Ok(())
}

