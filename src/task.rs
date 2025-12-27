use chrono::{DateTime, Duration, NaiveTime, TimeDelta, Utc};
use crate::schedule::Schedule;

pub struct Task {
    pub name: String,
    pub schedule: Schedule,
    pub completions: Option<Vec<Completion>>,
    pub alerting_time: Option<TimeDelta>,
}

#[derive(Copy, Clone)]
pub struct Completion {
    when: DateTime<Utc>, 
}

impl Task {
    pub fn is_due(&self) -> bool {
        self
            .last_completion()
            .map(|completion: Completion| completion.when < self.schedule.most_recent_due_date())
            .unwrap_or(true)
    }

    pub fn is_alerting(&self) -> bool {
        self
            .last_completion()
            .map(|completion: Completion| completion.when < (self.schedule.most_recent_due_date() - self.alerting_time.unwrap_or(Duration::zero())))
            .unwrap_or(true)
    }

    pub fn most_recent_due_date(&self) -> DateTime<Utc> {
        self.schedule.most_recent_due_date()
    }

    pub fn last_completion(&self) -> Option<Completion> {
        self.completions.as_ref().map(|completions| {
            let mut c = completions.iter().cloned().collect::<Vec<_>>();
            c.sort_by(|a, b| a.when.cmp(&b.when));
            completions.last().cloned()
        }).flatten()
    }
}
