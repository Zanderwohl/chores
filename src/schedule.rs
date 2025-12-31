use chrono::{DateTime, Datelike, Local, NaiveTime, Utc, Weekday};

pub struct Schedule {
    pub kind: ScheduleKind,
    // Keeps all variants around so switching schedule type can be switched back
    pub n_days: NDays,
    pub n_weeks: NWeeks,
    pub monthwise: Monthwise,
    pub weeks_of_month: WeeksOfMonth,
    pub certain_months: CertainMonths,
    pub once: Once,
}

impl Schedule {
    pub fn most_recent_due_date(&self) -> DateTime<Utc> {
        match self.kind {
            ScheduleKind::NDays => self.n_days.most_recent_due_date(),
            ScheduleKind::NWeeks => self.n_weeks.most_recent_due_date(),
            ScheduleKind::Monthwise => self.monthwise.most_recent_due_date(),
            ScheduleKind::WeeksOfMonth => self.weeks_of_month.most_recent_due_date(),
            ScheduleKind::CertainMonths => self.certain_months.most_recent_due_date(),
            ScheduleKind::Once => self.once.most_recent_due_date(),
        }
    }
}

#[derive(Clone)]
pub enum ScheduleKind {
    NDays,
    NWeeks,
    Monthwise,
    WeeksOfMonth,
    CertainMonths,
    Once,
}

// A one-time event at a specific date and time
#[derive(Clone)]
pub struct Once {
    pub datetime: DateTime<Utc>,
}

impl Once {
    pub fn most_recent_due_date(&self) -> DateTime<Utc> {
        self.datetime
    }
}

// Every so-and-so-many days, at a certain time.
#[derive(Clone)]
pub struct NDays {
    pub days: i32,
    pub time: NaiveTime,
}

impl NDays {
    pub(crate) fn most_recent_due_date(&self) -> DateTime<Utc> {
        let now = Utc::now();
        let local_now: DateTime<Local> = now.into();
        
        // Get today at the specified time
        let today_at_time = local_now
            .date_naive()
            .and_time(self.time)
            .and_local_timezone(Local)
            .unwrap()
            .with_timezone(&Utc);
        
        // If today at the specified time hasn't passed yet, go back by `days` days
        if today_at_time > now {
            today_at_time - chrono::Duration::days(self.days as i64)
        } else {
            today_at_time
        }
    }
}

// Every so-and-so-many weeks,
// e.g. Every other week on Tuesdays
// Or, every Tuesday and Thursday
#[derive(Clone)]
pub struct NWeeks {
    pub weeks: i32,
    pub sub_schedule: DaysOfWeek,
}

impl NWeeks {
    pub(crate) fn most_recent_due_date(&self) -> DateTime<Utc> {
        let now = Utc::now();
        let local_now: DateTime<Local> = now.into();
        let today = local_now.weekday();
        
        // Check if today is an active day and if the time has passed
        if self.sub_schedule.active(today) {
            let today_at_time = local_now
                .date_naive()
                .and_time(self.sub_schedule.time)
                .and_local_timezone(Local)
                .unwrap()
                .with_timezone(&Utc);
            
            if today_at_time <= now {
                return today_at_time;
            }
        }
        
        // Look backwards for the most recent active day
        for days_back in 1..=(7 * self.weeks) {
            let check_date = local_now - chrono::Duration::days(days_back as i64);
            if self.sub_schedule.active(check_date.weekday()) {
                return check_date
                    .date_naive()
                    .and_time(self.sub_schedule.time)
                    .and_local_timezone(Local)
                    .unwrap()
                    .with_timezone(&Utc);
            }
        }
        
        // Fallback to now if no valid date found
        now
    }
}

// On certain days of each month, e.g. 1st and 15th
// at a certain time
#[derive(Clone)]
pub struct Monthwise {
    pub days: Vec<i32>,
    pub time: NaiveTime,
}

impl Monthwise {
    pub(crate) fn most_recent_due_date(&self) -> DateTime<Utc> {
        let now = Utc::now();
        let local_now: DateTime<Local> = now.into();
        let today_day = local_now.day() as i32;
        
        // Check if today is one of the scheduled days and time has passed
        for &day in &self.days {
            if day == today_day {
                let today_at_time = local_now
                    .date_naive()
                    .and_time(self.time)
                    .and_local_timezone(Local)
                    .unwrap()
                    .with_timezone(&Utc);
                
                if today_at_time <= now {
                    return today_at_time;
                }
            }
        }
        
        // Find the most recent day in this month that's before today
        let mut most_recent_day = None;
        for &day in &self.days {
            if day < today_day {
                most_recent_day = Some(most_recent_day.map_or(day, |prev: i32| prev.max(day)));
            }
        }
        
        if let Some(day) = most_recent_day {
            return local_now
                .with_day(day as u32)
                .unwrap()
                .date_naive()
                .and_time(self.time)
                .and_local_timezone(Local)
                .unwrap()
                .with_timezone(&Utc);
        }
        
        // Otherwise, look at the previous month
        let prev_month = local_now - chrono::Duration::days(28);
        let last_day_of_prev_month = prev_month
            .with_day(1)
            .unwrap()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .unwrap()
            - chrono::Duration::days(1);
        
        let max_day_prev = last_day_of_prev_month.day() as i32;
        let mut most_recent_day_prev = None;
        for &day in &self.days {
            if day <= max_day_prev {
                most_recent_day_prev = Some(most_recent_day_prev.map_or(day, |prev: i32| prev.max(day)));
            }
        }
        
        if let Some(day) = most_recent_day_prev {
            return last_day_of_prev_month
                .with_day(day as u32)
                .unwrap()
                .date_naive()
                .and_time(self.time)
                .and_local_timezone(Local)
                .unwrap()
                .with_timezone(&Utc);
        }
        
        now
    }
}

// On certain nth weekdays,
// e.g. Every 2nd and 3rd Tuesday
// or every Tuesday and Thursday except if it's the fifth week of the month
#[derive(Clone)]
pub struct WeeksOfMonth {
    pub weeks: Vec<i32>,
    pub sub_schedule: DaysOfWeek,
}

// On certain days of certain months,
// e.g. the 15th and 20th of February and March
#[derive(Clone)]
pub struct CertainMonths {
    pub months: Vec<i32>, // 1-12 for Jan-Dec
    pub days: Vec<i32>,   // 1-31 for days of month
    pub time: NaiveTime,
}

impl WeeksOfMonth {
    pub(crate) fn most_recent_due_date(&self) -> DateTime<Utc> {
        let now = Utc::now();
        let local: DateTime<Local> = now.into();
        let today = local.weekday();
        
        // Helper function to get the week number of a date in the month (1-5)
        let get_week_of_month = |date: &DateTime<Local>| -> i32 {
            ((date.day() - 1) / 7 + 1) as i32
        };
        
        let current_week = get_week_of_month(&local);
        
        // Check if today matches the pattern and time has passed
        if self.sub_schedule.active(today) && self.weeks.contains(&current_week) {
            let today_at_time = local
                .date_naive()
                .and_time(self.sub_schedule.time)
                .and_local_timezone(Local)
                .unwrap()
                .with_timezone(&Utc);
            
            if today_at_time <= now {
                return today_at_time;
            }
        }
        
        // Look backwards through days to find the most recent matching date
        for days_back in 1..=60 {
            let check_date = local - chrono::Duration::days(days_back as i64);
            let week_num = get_week_of_month(&check_date);
            
            if self.sub_schedule.active(check_date.weekday()) && self.weeks.contains(&week_num) {
                return check_date
                    .date_naive()
                    .and_time(self.sub_schedule.time)
                    .and_local_timezone(Local)
                    .unwrap()
                    .with_timezone(&Utc);
            }
        }
        
        now
    }
}

impl CertainMonths {
    pub(crate) fn most_recent_due_date(&self) -> DateTime<Utc> {
        let now = Utc::now();
        let local_now: DateTime<Local> = now.into();
        let current_month = local_now.month() as i32;
        let current_day = local_now.day() as i32;
        
        // Check if today is a matching day in a matching month and time has passed
        if self.months.contains(&current_month) && self.days.contains(&current_day) {
            let today_at_time = local_now
                .date_naive()
                .and_time(self.time)
                .and_local_timezone(Local)
                .unwrap()
                .with_timezone(&Utc);
            
            if today_at_time <= now {
                return today_at_time;
            }
        }
        
        // Look backwards through days to find the most recent matching date
        // Look back up to 365 days since months might be spread throughout the year
        for days_back in 1..=365 {
            let check_date = local_now - chrono::Duration::days(days_back as i64);
            let check_month = check_date.month() as i32;
            let check_day = check_date.day() as i32;
            
            if self.months.contains(&check_month) && self.days.contains(&check_day) {
                return check_date
                    .date_naive()
                    .and_time(self.time)
                    .and_local_timezone(Local)
                    .unwrap()
                    .with_timezone(&Utc);
            }
        }
        
        now
    }
}

#[derive(Clone)]
pub struct DaysOfWeek {
    pub sunday: bool,
    pub monday: bool,
    pub tuesday: bool,
    pub wednesday: bool,
    pub thursday: bool,
    pub friday: bool,
    pub saturday: bool,
    pub time: NaiveTime,
}

impl DaysOfWeek {
    pub fn active(&self, day: Weekday) -> bool {
        match day {
            Weekday::Sun => self.sunday,
            Weekday::Mon => self.monday,
            Weekday::Tue => self.tuesday,
            Weekday::Wed => self.wednesday,
            Weekday::Thu => self.thursday,
            Weekday::Fri => self.friday,
            Weekday::Sat => self.saturday,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Timelike};

    #[test]
    fn test_ndays_basic() {
        // Test every 3 days at 10:00 AM
        let schedule = NDays {
            days: 3,
            time: NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
        };

        let result = schedule.most_recent_due_date();
        let local_result: DateTime<Local> = result.into();
        
        // Should return a date at 10:00 AM
        assert_eq!(local_result.time().hour(), 10);
        assert_eq!(local_result.time().minute(), 0);
        
        // Result should be in the past or today
        assert!(result <= Utc::now());
    }

    #[test]
    fn test_ndays_single_day() {
        // Test every day at noon
        let schedule = NDays {
            days: 1,
            time: NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
        };

        let result = schedule.most_recent_due_date();
        let local_result: DateTime<Local> = result.into();
        
        // Should return noon
        assert_eq!(local_result.time().hour(), 12);
        assert_eq!(local_result.time().minute(), 0);
        
        // Should be today at noon or yesterday at noon depending on current time
        let now_local: DateTime<Local> = Utc::now().into();
        let today_noon = now_local
            .date_naive()
            .and_time(NaiveTime::from_hms_opt(12, 0, 0).unwrap())
            .and_local_timezone(Local)
            .unwrap();
        
        if Utc::now() >= today_noon.with_timezone(&Utc) {
            // If it's past noon, should return today at noon
            assert_eq!(local_result.date_naive(), now_local.date_naive());
        } else {
            // If it's before noon, should return yesterday at noon
            assert_eq!(
                local_result.date_naive(),
                (now_local - Duration::days(1)).date_naive()
            );
        }
    }

    #[test]
    fn test_nweeks_single_day() {
        // Test every week on Mondays at 9:00 AM
        let schedule = NWeeks {
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
        };

        let result = schedule.most_recent_due_date();
        let local_result: DateTime<Local> = result.into();
        
        // Should return 9:00 AM on a Monday
        assert_eq!(local_result.time().hour(), 9);
        assert_eq!(local_result.time().minute(), 0);
        assert_eq!(local_result.weekday(), Weekday::Mon);
        
        // Should be in the past or today
        assert!(result <= Utc::now());
    }

    #[test]
    fn test_nweeks_multiple_days() {
        // Test every week on Tuesdays and Thursdays at 2:00 PM
        let schedule = NWeeks {
            weeks: 1,
            sub_schedule: DaysOfWeek {
                sunday: false,
                monday: false,
                tuesday: true,
                wednesday: false,
                thursday: true,
                friday: false,
                saturday: false,
                time: NaiveTime::from_hms_opt(14, 0, 0).unwrap(),
            },
        };

        let result = schedule.most_recent_due_date();
        let local_result: DateTime<Local> = result.into();
        
        // Should return 2:00 PM
        assert_eq!(local_result.time().hour(), 14);
        assert_eq!(local_result.time().minute(), 0);
        
        // Should be either Tuesday or Thursday
        let weekday = local_result.weekday();
        assert!(weekday == Weekday::Tue || weekday == Weekday::Thu);
        
        // Should be in the past or today
        assert!(result <= Utc::now());
    }

    #[test]
    fn test_nweeks_every_other_week() {
        // Test every other week on Wednesdays at 11:00 AM
        let schedule = NWeeks {
            weeks: 2,
            sub_schedule: DaysOfWeek {
                sunday: false,
                monday: false,
                tuesday: false,
                wednesday: true,
                thursday: false,
                friday: false,
                saturday: false,
                time: NaiveTime::from_hms_opt(11, 0, 0).unwrap(),
            },
        };

        let result = schedule.most_recent_due_date();
        let local_result: DateTime<Local> = result.into();
        
        // Should return 11:00 AM on a Wednesday
        assert_eq!(local_result.time().hour(), 11);
        assert_eq!(local_result.time().minute(), 0);
        assert_eq!(local_result.weekday(), Weekday::Wed);
        
        // Should be in the past or today
        assert!(result <= Utc::now());
        
        // Should be within the last 14 days
        let days_ago = (Utc::now() - result).num_days();
        assert!(days_ago <= 14);
    }

    #[test]
    fn test_monthwise_single_day() {
        // Test on the 1st of each month at 8:00 AM
        let schedule = Monthwise {
            days: vec![1],
            time: NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
        };

        let result = schedule.most_recent_due_date();
        let local_result: DateTime<Local> = result.into();
        
        // Should return 8:00 AM on the 1st
        assert_eq!(local_result.time().hour(), 8);
        assert_eq!(local_result.time().minute(), 0);
        assert_eq!(local_result.day(), 1);
        
        // Should be in the past or today
        assert!(result <= Utc::now());
    }

    #[test]
    fn test_monthwise_multiple_days() {
        // Test on the 1st and 15th of each month at 3:00 PM
        let schedule = Monthwise {
            days: vec![1, 15],
            time: NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
        };

        let result = schedule.most_recent_due_date();
        let local_result: DateTime<Local> = result.into();
        
        // Should return 3:00 PM
        assert_eq!(local_result.time().hour(), 15);
        assert_eq!(local_result.time().minute(), 0);
        
        // Should be either 1st or 15th
        let day = local_result.day();
        assert!(day == 1 || day == 15);
        
        // Should be in the past or today
        assert!(result <= Utc::now());
    }

    #[test]
    fn test_monthwise_mid_month() {
        // Test on the 10th, 20th, and 25th at 10:30 AM
        let schedule = Monthwise {
            days: vec![10, 20, 25],
            time: NaiveTime::from_hms_opt(10, 30, 0).unwrap(),
        };

        let result = schedule.most_recent_due_date();
        let local_result: DateTime<Local> = result.into();
        
        // Should return 10:30 AM
        assert_eq!(local_result.time().hour(), 10);
        assert_eq!(local_result.time().minute(), 30);
        
        // Should be one of the scheduled days
        let day = local_result.day();
        assert!(day == 10 || day == 20 || day == 25);
        
        // Should be in the past or today
        assert!(result <= Utc::now());
    }

    #[test]
    fn test_weeks_of_month_first_monday() {
        // Test every 1st Monday of the month at 9:00 AM
        let schedule = WeeksOfMonth {
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
        };

        let result = schedule.most_recent_due_date();
        let local_result: DateTime<Local> = result.into();
        
        // Should return 9:00 AM on a Monday
        assert_eq!(local_result.time().hour(), 9);
        assert_eq!(local_result.time().minute(), 0);
        assert_eq!(local_result.weekday(), Weekday::Mon);
        
        // Should be in the first week of the month (days 1-7)
        let day = local_result.day();
        assert!(day >= 1 && day <= 7);
        
        // Should be in the past or today
        assert!(result <= Utc::now());
    }

    #[test]
    fn test_weeks_of_month_second_and_fourth_friday() {
        // Test 2nd and 4th Friday of the month at 5:00 PM
        let schedule = WeeksOfMonth {
            weeks: vec![2, 4],
            sub_schedule: DaysOfWeek {
                sunday: false,
                monday: false,
                tuesday: false,
                wednesday: false,
                thursday: false,
                friday: true,
                saturday: false,
                time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
            },
        };

        let result = schedule.most_recent_due_date();
        let local_result: DateTime<Local> = result.into();
        
        // Should return 5:00 PM on a Friday
        assert_eq!(local_result.time().hour(), 17);
        assert_eq!(local_result.time().minute(), 0);
        assert_eq!(local_result.weekday(), Weekday::Fri);
        
        // Should be in the 2nd or 4th week (days 8-14 or 22-28)
        let day = local_result.day();
        assert!((day >= 8 && day <= 14) || (day >= 22 && day <= 28));
        
        // Should be in the past or today
        assert!(result <= Utc::now());
    }

    #[test]
    fn test_weeks_of_month_multiple_weekdays() {
        // Test 1st and 3rd Tuesday and Thursday at 1:00 PM
        let schedule = WeeksOfMonth {
            weeks: vec![1, 3],
            sub_schedule: DaysOfWeek {
                sunday: false,
                monday: false,
                tuesday: true,
                wednesday: false,
                thursday: true,
                friday: false,
                saturday: false,
                time: NaiveTime::from_hms_opt(13, 0, 0).unwrap(),
            },
        };

        let result = schedule.most_recent_due_date();
        let local_result: DateTime<Local> = result.into();
        
        // Should return 1:00 PM
        assert_eq!(local_result.time().hour(), 13);
        assert_eq!(local_result.time().minute(), 0);
        
        // Should be Tuesday or Thursday
        let weekday = local_result.weekday();
        assert!(weekday == Weekday::Tue || weekday == Weekday::Thu);
        
        // Should be in the 1st or 3rd week (days 1-7 or 15-21)
        let day = local_result.day();
        assert!((day >= 1 && day <= 7) || (day >= 15 && day <= 21));
        
        // Should be in the past or today
        assert!(result <= Utc::now());
    }

    #[test]
    fn test_days_of_week_active() {
        let schedule = DaysOfWeek {
            sunday: true,
            monday: false,
            tuesday: true,
            wednesday: false,
            thursday: true,
            friday: false,
            saturday: true,
            time: NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
        };

        assert!(schedule.active(Weekday::Sun));
        assert!(!schedule.active(Weekday::Mon));
        assert!(schedule.active(Weekday::Tue));
        assert!(!schedule.active(Weekday::Wed));
        assert!(schedule.active(Weekday::Thu));
        assert!(!schedule.active(Weekday::Fri));
        assert!(schedule.active(Weekday::Sat));
    }

    #[test]
    fn test_ndays_weekly() {
        // Test every 7 days (weekly) at 6:00 PM
        let schedule = NDays {
            days: 7,
            time: NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
        };

        let result = schedule.most_recent_due_date();
        let local_result: DateTime<Local> = result.into();
        
        // Should return 6:00 PM
        assert_eq!(local_result.time().hour(), 18);
        assert_eq!(local_result.time().minute(), 0);
        
        // Should be in the past or today
        assert!(result <= Utc::now());
    }

    #[test]
    fn test_monthwise_end_of_month() {
        // Test on the 28th, 29th, 30th at 11:00 PM
        // Note: Not all months have 30 days, but the function should handle this
        let schedule = Monthwise {
            days: vec![28, 29, 30],
            time: NaiveTime::from_hms_opt(23, 0, 0).unwrap(),
        };

        let result = schedule.most_recent_due_date();
        let local_result: DateTime<Local> = result.into();
        
        // Should return 11:00 PM
        assert_eq!(local_result.time().hour(), 23);
        assert_eq!(local_result.time().minute(), 0);
        
        // Should be one of the scheduled days (if valid for that month)
        let day = local_result.day();
        assert!(day >= 28 && day <= 30);
        
        // Should be in the past or today
        assert!(result <= Utc::now());
    }
}