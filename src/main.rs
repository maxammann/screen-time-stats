use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, Utc};
use crossterm::event::{self, Event, KeyCode};
use ratatui::widgets::Paragraph;
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, Tabs},
};
use rusqlite::Connection;
use std::cmp::Reverse;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::fs;
use std::io::{self};
use std::path::Path;

const KNOWLEDGE_DB: &str = "/Users/max/Library/Application Support/Knowledge/knowledgeC.db";

#[derive(Debug)]
struct UsageData {
    app: String,
    //device_id: String,
    //device_model: String,
    usage: i64,
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
    //created_at: DateTime<Utc>,
    //tz: f64,
}
fn query_database() -> anyhow::Result<Vec<UsageData>> {
    let db_path = KNOWLEDGE_DB;

    if !Path::new(&db_path).exists() {
        eprintln!("Could not find knowledgeC.db at {}.", db_path);
        std::process::exit(1);
    }

    if fs::metadata(&db_path).is_err() {
        eprintln!("The knowledgeC.db at {} is not readable.\nPlease grant full disk access to the application running the script.", db_path);
        std::process::exit(1);
    }

    let conn = Connection::open(db_path)?;
    let mut stmt = conn.prepare(
        r#"
        SELECT
            ZOBJECT.ZVALUESTRING AS "app",
            (ZOBJECT.ZENDDATE - ZOBJECT.ZSTARTDATE) AS "usage",
            (ZOBJECT.ZSTARTDATE + 978307200) as "start_time",
            (ZOBJECT.ZENDDATE + 978307200) as "end_time",
            (ZOBJECT.ZCREATIONDATE + 978307200) as "created_at",
            ZOBJECT.ZSECONDSFROMGMT AS "tz",
            ZSOURCE.ZDEVICEID AS "device_id",
            ZMODEL AS "device_model"
        FROM
            ZOBJECT
            LEFT JOIN
            ZSTRUCTUREDMETADATA
            ON ZOBJECT.ZSTRUCTUREDMETADATA = ZSTRUCTUREDMETADATA.Z_PK
            LEFT JOIN
            ZSOURCE
            ON ZOBJECT.ZSOURCE = ZSOURCE.Z_PK
            LEFT JOIN
            ZSYNCPEER
            ON ZSOURCE.ZDEVICEID = ZSYNCPEER.ZDEVICEID
        WHERE
            ZSTREAMNAME = "/app/usage"
        ORDER BY
            ZSTARTDATE DESC
        "#,
    )?;

    let rows = stmt.query_map([], |row| {
        let start_time = DateTime::from_timestamp(row.get(2)?, 0).unwrap_or_default();
        let end_time = DateTime::from_timestamp(row.get(3)?, 0).unwrap_or_default();
        //let created_at = DateTime::from_timestamp(row.get::<_, f64>(4)? as i64, 0).unwrap_or_default();
        // let tz = row.get(5)?;
        Ok(UsageData {
            app: row.get(0)?,
            usage: row.get(1)?,
            start_time,
            end_time,
            //created_at,
            //tz,
            //device_id: row.get::<_, Option<String>>(6)?.unwrap_or_else(|| "Unknown".to_string()),
            //device_model: row.get::<_, Option<String>>(7)?.unwrap_or_else(|| "Unknown".to_string()),
        })
    })?;

    let results = rows.collect::<Result<Vec<UsageData>, _>>()?;
    Ok(results)
}

fn format_duration(duration: &Duration) -> String {
    let seconds = duration.num_seconds();
    let minutes = seconds / 60;
    let hours = minutes / 60;

    if hours > 0 {
        format!("{}h {}min", hours, minutes % 60)
    } else if minutes > 0 {
        format!("{}min {}s", minutes, seconds % 60)
    } else {
        format!("{}s", seconds)
    }
}

#[derive(Debug, Default)]
struct DailyUsage {
    total_usage: i64,
    first_usage: DateTime<Local>,
    last_usage: DateTime<Local>,
    per_app_usage: HashMap<String, i64>,
    breaks: Vec<(DateTime<Local>, DateTime<Local>, Duration)>, // Global break intervals per day with duration
    net_active_time: Duration,
}

impl Display for DailyUsage {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Date: {}", self.first_usage.date_naive())?;
        writeln!(
            f,
            "  Total Usage: {}",
            format_duration(&Duration::seconds(self.total_usage))
        )?;
        writeln!(f, "  First Usage: {}", self.first_usage)?;
        writeln!(f, "  Last Usage: {}", self.last_usage)?;
        writeln!(
            f,
            "  Net Active Hours: {}",
            format_duration(&self.net_active_time)
        )?;
        writeln!(f, "  Per App Usage:")?;
        for (app, usage_time) in &self.per_app_usage {
            writeln!(
                f,
                "    {}: {}",
                app,
                format_duration(&Duration::seconds(*usage_time))
            )?;
        }
        writeln!(f, "  Breaks:")?;
        for (start, end, duration) in &self.breaks {
            writeln!(
                f,
                "    Break from {} to {} ({})",
                start,
                end,
                format_duration(&duration)
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
struct WeeklyUsage {
    total_usage: i64,
    net_active_hours: Duration,
    per_app_usage: HashMap<String, i64>,
    first_day: NaiveDate,
    is_current_week: bool,
}

impl Display for WeeklyUsage {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let current_marker = if self.is_current_week {
            "(Current Week)"
        } else {
            ""
        };
        writeln!(
            f,
            "Week {} (Starting {}) {}:",
            self.first_day.iso_week().week(),
            self.first_day,
            current_marker
        )?;
        writeln!(
            f,
            "  Total Usage: {}",
            format_duration(&Duration::seconds(self.total_usage))
        )?;
        writeln!(
            f,
            "  Net Active Time: {}",
            format_duration(&self.net_active_hours)
        )?;
        writeln!(f, "  Per App Usage:")?;
        for (app, usage_time) in &self.per_app_usage {
            writeln!(
                f,
                "    {}: {}",
                app,
                format_duration(&Duration::seconds(*usage_time))
            )?;
        }

        Ok(())
    }
}

fn analyze_usage(data: Vec<UsageData>) -> Vec<(NaiveDate, DailyUsage)> {
    let mut daily_usage: HashMap<NaiveDate, DailyUsage> = HashMap::new();

    for entry in &data {
        let date = entry.start_time.date_naive();
        let first_usage = entry.start_time.with_timezone(&Local);
        let last_usage = entry.end_time.with_timezone(&Local);

        let daily_entry = daily_usage.entry(date).or_insert_with(|| DailyUsage {
            first_usage,
            last_usage,
            ..Default::default()
        });

        daily_entry.total_usage += entry.usage;
        *daily_entry
            .per_app_usage
            .entry(entry.app.clone())
            .or_insert(0) += entry.usage;

        if first_usage < daily_entry.first_usage {
            daily_entry.first_usage = first_usage;
        }
        if last_usage > daily_entry.last_usage {
            daily_entry.last_usage = last_usage;
        }
    }

    for (date, usage) in daily_usage.iter_mut() {
        let mut sessions: Vec<(DateTime<Local>, DateTime<Local>)> = Vec::new();

        for entry in &data {
            if entry.start_time.date_naive() == *date {
                sessions.push((
                    entry.start_time.with_timezone(&Local),
                    entry.end_time.with_timezone(&Local),
                ));
            }
        }

        sessions.sort_by_key(|(start, _)| *start);
        let mut breaks = Vec::new();
        let mut total_break_duration = Duration::zero();

        for i in 1..sessions.len() {
            let (_, prev_end) = sessions[i - 1];
            let (current_start, _) = sessions[i];
            let break_duration = current_start.signed_duration_since(prev_end);
            if break_duration > Duration::minutes(10) {
                breaks.push((prev_end, current_start, break_duration));
                total_break_duration += break_duration;
            }
        }

        usage.breaks = breaks;
        usage.net_active_time =
            usage.last_usage.signed_duration_since(usage.first_usage) - total_break_duration;
    }

    let mut sorted_analysis: Vec<_> = daily_usage.into_iter().collect();
    sorted_analysis.sort_by_key(|(date, _)| Reverse(*date));
    sorted_analysis
}

fn analyze_weekly_usage(daily_usage: &Vec<(NaiveDate, DailyUsage)>) -> Vec<(u32, WeeklyUsage)> {
    let mut weekly_usage: HashMap<u32, WeeklyUsage> = HashMap::new();
    let current_week = Local::now().iso_week().week();

    for (date, usage) in daily_usage {
        let week = date.iso_week().week();
        let first_day = *date - Duration::days(date.weekday().num_days_from_monday() as i64);
        let is_current_week = week == current_week;
        let weekly_entry = weekly_usage.entry(week).or_insert_with(|| WeeklyUsage {
            first_day,
            is_current_week,
            ..Default::default()
        });

        weekly_entry.total_usage += usage.total_usage;
        weekly_entry.net_active_hours += usage.net_active_time;

        for (app, time) in &usage.per_app_usage {
            *weekly_entry.per_app_usage.entry(app.clone()).or_insert(0) += time;
        }
    }

    let mut sorted_analysis: Vec<_> = weekly_usage.into_iter().collect();
    sorted_analysis.sort_by_key(|(week, _)| Reverse(*week));
    sorted_analysis
}

fn generate_entries_and_details<U: Display, V: Display, F>(
    analysis: &Vec<(U, V)>,
    selected_index: usize,
    list_item_name: F,
) -> (Vec<ListItem>, String)
where
    F: Fn(&U, &V) -> String,
{
    let entries: Vec<_> = analysis
        .iter()
        .enumerate()
        .map(|(i, (key, value))| {
            let style = if i == selected_index {
                Style::default().fg(Color::Green)
            } else {
                Style::default()
            };
            ListItem::new(list_item_name(key, value)).style(style)
        })
        .collect();

    let detail = analysis
        .get(selected_index)
        .map(|(_, usage)| format!("{}", usage))
        .unwrap_or("No data available".to_string());

    (entries, detail)
}

fn run_tui(
    daily_analysis: Vec<(NaiveDate, DailyUsage)>,
    weekly_analysis: Vec<(u32, WeeklyUsage)>,
) -> Result<(), io::Error> {
    let mut terminal = ratatui::init();

    let mut selected_tab = 0;
    let mut selected_index = 0;

    loop {
        terminal.draw(|frame| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints([Constraint::Length(3), Constraint::Min(0)].as_ref())
                .split(frame.area());

            let titles = vec!["Daily Analysis", "Weekly Analysis"];
            let tabs = Tabs::new(titles)
                .block(Block::default().borders(Borders::ALL).title("Analysis"))
                .select(selected_tab)
                .highlight_style(Style::default().fg(Color::Yellow));
            frame.render_widget(tabs, chunks[0]);

            let (items, details) = match selected_tab {
                0 => generate_entries_and_details(&daily_analysis, selected_index, |key, value| {
                    key.to_string()
                }),
                _ => {
                    generate_entries_and_details(&weekly_analysis, selected_index, |key, value| {
                        format!("Week {} (Starting {})", key, value.first_day)
                    })
                }
            };

            let layout = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
                .split(chunks[1]);

            let list =
                List::new(items).block(Block::default().borders(Borders::ALL).title("Entries"));
            frame.render_widget(list, layout[0]);

            let detail_widget = Paragraph::new(details)
                .block(Block::default().borders(Borders::ALL).title("Details"));
            frame.render_widget(detail_widget, layout[1]);
        })?;

        if let Ok(event) = event::read() {
            if let Event::Key(key) = event {
                match key.code {
                    KeyCode::Left => {
                        selected_tab = 0;
                        selected_index = 0;
                    }
                    KeyCode::Right => {
                        selected_tab = 1;
                        selected_index = 0;
                    }
                    KeyCode::Up => {
                        if selected_index > 0 {
                            selected_index -= 1;
                        }
                    }
                    KeyCode::Down => {
                        let max_index = match selected_tab {
                            0 => daily_analysis.len().saturating_sub(1),
                            _ => weekly_analysis.len().saturating_sub(1),
                        };
                        if selected_index < max_index {
                            selected_index += 1;
                        }
                    }
                    KeyCode::Esc | KeyCode::Char('q') => break,
                    _ => {}
                }
            }
        }
    }

    ratatui::restore();
    Ok(())
}

fn main() {
    match query_database() {
        Ok(data) => {
            let daily_analysis = analyze_usage(data);
            let weekly_analysis = analyze_weekly_usage(&daily_analysis);
            if let Err(e) = run_tui(daily_analysis, weekly_analysis) {
                eprintln!("TUI Error: {:?}", e);
            }
        }
        Err(e) => eprintln!("Error querying database: {:?}", e),
    }
}
