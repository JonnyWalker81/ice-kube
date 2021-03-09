use anyhow::Result;
use clap::Clap;
use futures::{StreamExt, TryStreamExt};
use k8s_openapi::api::core::v1::Pod;
use kube::{
    api::{Api, ListParams, LogParams, Meta},
    Client, Config,
};
// use log::info;
use crossterm::{
    event::{self, Event as CEvent, KeyCode},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use futures::future::join_all;
use lazy_static::lazy_static;
use regex::Regex;
use std::io::Write;
use std::{collections::HashMap, time::Duration};
use std::{io, time::Instant};
use termion::{color, style};
use tokio::io::{self as tok_io, AsyncWriteExt};
use tokio::task;
use tui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{
        Block, BorderType, Borders, Cell, List, ListItem, ListState, Paragraph, Row, Table, Tabs,
    },
    Terminal,
};

lazy_static! {
    static ref COLORS: Vec<color::Rgb> = vec![
        color::Rgb(0, 255, 0),
        color::Rgb(128, 128, 0),
        color::Rgb(0, 255, 255),
        color::Rgb(255, 192, 203),
        color::Rgb(245, 255, 250),
        color::Rgb(221, 160, 221),
        color::Rgb(154, 205, 50),
        color::Rgb(230, 230, 250),
    ];
}

// #[derive(Error, Debug)]
// pub enum Error {
//     #[error("error with Kubernetes: {0}")]
//     KubeError(#[from] io::Error),
//     #[error("error parsing the DB file: {0}")]
//     ParseDBError(#[from] serde_json::Error),
// }

#[derive(Clone, Debug)]
enum Event<I> {
    Input(I),
    Tick,
}

#[derive(Copy, Clone, Debug)]
enum MenuItem {
    Home,
}

impl From<MenuItem> for usize {
    fn from(input: MenuItem) -> usize {
        match input {
            MenuItem::Home => 0,
        }
    }
}

#[derive(Clone, Debug)]
struct KubePod {
    name: String,
}

#[derive(Clap)]
#[clap(version = "0.1.0", author = "Jonathan Rothberg")]
struct Opts {
    #[clap(subcommand)]
    subcmd: SubCmd,
}

#[derive(Clap)]
enum SubCmd {
    #[clap(name = "logs")]
    Logs(LogsOpts),
    #[clap(name = "ui")]
    UI(UIOpts),
}

#[derive(Debug, Clap)]
struct LogsOpts {
    #[clap(long = "pod")]
    pod: Option<String>,
    #[clap(short = 'f')]
    follow: bool,
    #[clap(short = 'n', default_value = "nuwolf")]
    namespace: String,
    #[clap(short = 't', long = "tail-length", default_value = "100")]
    tail_length: i64,
    #[clap(short = 'p', long = "pattern")]
    pattern: Option<String>,
    #[clap(short = 'r', long = "terms")]
    terms: Option<String>,
}

#[derive(Debug, Clap)]
struct UIOpts {
    #[clap(short = 'n', default_value = "nuwolf")]
    namespace: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let opts: Opts = Opts::parse();

    match opts.subcmd {
        SubCmd::Logs(o) => match o.pod {
            Some(p) => {
                let c = get_color()?;
                stream_logs(o.namespace, p, o.tail_length, c).await?;
            }
            None => match o.pattern {
                Some(ref p) => {
                    follow_logs(&o, &p).await?;
                }
                None => {
                    select_pod(&o).await?;
                }
            },
        },
        SubCmd::UI(o) => {
            load_ui(&o.namespace, &o).await?;
        }
    }

    Ok(())
}

fn get_color() -> Result<color::Rgb> {
    use rand::Rng;

    let mut rng = rand::thread_rng();
    let index = rng.gen_range(0.0, COLORS.len() as f32) as usize;
    let rgb = COLORS[index];

    Ok(rgb)
}

async fn load_ui(namespace: &str, opts: &UIOpts) -> Result<()> {
    println!("Loading UI...");
    enable_raw_mode().expect("can run in raw mode");

    let (mut tx, mut rx) = tokio::sync::mpsc::channel(1);
    let tick_rate = Duration::from_millis(200);
    tokio::spawn(async move {
        let mut last_tick = Instant::now();
        loop {
            let timeout = tick_rate
                .checked_sub(last_tick.elapsed())
                .unwrap_or_else(|| Duration::from_secs(0));

            if event::poll(timeout).expect("poll works") {
                if let CEvent::Key(key) = event::read().expect("can read events") {
                    tx.send(Event::Input(key)).await;
                }
            }

            if last_tick.elapsed() >= tick_rate {
                if let Ok(_) = tx.send(Event::Tick).await {
                    last_tick = Instant::now();
                }
            }
        }
    });

    let stdout = io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let menu_titles = vec!["Home"];
    let mut active_menu_item = MenuItem::Home;
    let mut pod_list_state = ListState::default();
    pod_list_state.select(Some(0));

    loop {
        let pod_list: Vec<_> = get_pods(namespace)
            .await?
            .iter()
            .map(|p| KubePod {
                name: Meta::name(p),
            })
            .collect();

        terminal.draw(|rect| {
            let size = rect.size();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(2)
                .constraints(
                    [
                        Constraint::Length(3),
                        Constraint::Min(2),
                        Constraint::Length(3),
                    ]
                    .as_ref(),
                )
                .split(size);

            let copyright = Paragraph::new("ice-kube 2021")
                .style(Style::default().fg(Color::LightCyan))
                .alignment(Alignment::Center)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .style(Style::default().fg(Color::White))
                        .title("Copyright")
                        .border_type(BorderType::Plain),
                );

            let menu = menu_titles
                .iter()
                .map(|t| {
                    let (first, rest) = t.split_at(1);
                    Spans::from(vec![
                        Span::styled(
                            first,
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::UNDERLINED),
                        ),
                        Span::styled(rest, Style::default().fg(Color::White)),
                    ])
                })
                .collect();

            let tabs = Tabs::new(menu)
                .select(active_menu_item.into())
                .block(Block::default().title("Menu").borders(Borders::ALL))
                .style(Style::default().fg(Color::White))
                .highlight_style(Style::default().fg(Color::Yellow))
                .divider(Span::raw("|"));

            rect.render_widget(tabs, chunks[0]);
            match active_menu_item {
                MenuItem::Home => {
                    let pods_chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints(
                            [Constraint::Percentage(20), Constraint::Percentage(80)].as_ref(),
                        )
                        .split(chunks[1]);
                    let (left, right) = render_pods(namespace, &pod_list, &pod_list_state);

                    rect.render_stateful_widget(left, pods_chunks[0], &mut pod_list_state);
                    rect.render_widget(right, pods_chunks[1]);
                }
            }
            rect.render_widget(copyright, chunks[2]);
        })?;

        if let Some(event) = rx.recv().await {
            match event {
                Event::Input(event) => match event.code {
                    KeyCode::Char('q') => {
                        disable_raw_mode()?;
                        terminal.show_cursor()?;
                        break;
                    }
                    // KeyCode::Char('h') => active_menu_item = MenuItem::Home,
                    // KeyCode::Char('p') => active_menu_item = MenuItem::Pets,
                    // KeyCode::Char('a') => {
                    //     add_random_pet_to_db().expect("can add new random pet");
                    // }
                    // KeyCode::Char('d') => {
                    //     remove_pet_at_index(&mut pet_list_state).expect("can remove pet");
                    // }
                    // KeyCode::Down => {
                    //     if let Some(selected) = pet_list_state.selected() {
                    //         let amount_pets = read_db().expect("can fetch pet list").len();
                    //         if selected >= amount_pets - 1 {
                    //             pet_list_state.select(Some(0));
                    //         } else {
                    //             pet_list_state.select(Some(selected + 1));
                    //         }
                    //     }
                    // }
                    // KeyCode::Up => {
                    //     if let Some(selected) = pet_list_state.selected() {
                    //         let amount_pets = read_db().expect("can fetch pet list").len();
                    //         if selected > 0 {
                    //             pet_list_state.select(Some(selected - 1));
                    //         } else {
                    //             pet_list_state.select(Some(amount_pets - 1));
                    //         }
                    //     }
                    // }
                    _ => {}
                },
                Event::Tick => {}
            }
        }
    }

    Ok(())
}

fn render_pods<'a>(
    namespace: &str,
    pod_list: &[KubePod],
    pod_list_state: &ListState,
) -> (List<'a>, Table<'a>) {
    let pods = Block::default()
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::White))
        .title("Pods")
        .border_type(BorderType::Plain);

    let items: Vec<_> = pod_list
        .iter()
        .map(|pod| {
            ListItem::new(Spans::from(vec![Span::styled(
                pod.name.clone(),
                Style::default(),
            )]))
        })
        .collect();

    let selected_pod = pod_list
        .get(
            pod_list_state
                .selected()
                .expect("there is always a selected pod"),
        )
        .expect("exists")
        .clone();

    let list = List::new(items).block(pods).highlight_style(
        Style::default()
            .bg(Color::Yellow)
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD),
    );

    let pod_detail = Table::new(vec![Row::new(vec![
        Cell::from(Span::raw(selected_pod.name)),
        Cell::from(Span::raw("1/1")),
        Cell::from(Span::raw("0")),
        Cell::from(Span::raw("Running")),
    ])])
    .header(Row::new(vec![
        Cell::from(Span::styled(
            "Name",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            "Ready",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            "Restarts",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            "Status",
            Style::default().add_modifier(Modifier::BOLD),
        )),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::White))
            .title("Detail")
            .border_type(BorderType::Plain),
    )
    .widths(&[
        Constraint::Percentage(20),
        Constraint::Percentage(5),
        Constraint::Percentage(5),
        Constraint::Percentage(70),
    ]);

    (list, pod_detail)
}

async fn follow_logs(o: &LogsOpts, p: &str) -> Result<()> {
    let pods = collect_pods(&o.namespace, &p).await?;
    println!("Pods: {:?}", pods);
    let mut tasks = vec![];
    for name in pods {
        let c = get_color()?;
        let n = o.namespace.clone();
        let pn = name.clone();
        let t = task::spawn(stream_logs(n, pn, o.tail_length, c));
        tasks.push(t);
    }

    join_all(tasks).await;

    Ok(())
}

async fn select_pod(o: &LogsOpts) -> Result<()> {
    // println!("Ops: {:?}", o);
    let pods = list_pods(&o.namespace).await?;
    let mut input = String::new();
    print!("Enter Pod number: ");
    io::stdout().flush()?;
    match io::stdin().read_line(&mut input) {
        Ok(_) => {
            let index = input.trim().parse::<usize>()?;
            if let Some(p) = pods.get(&index) {
                println!("{}", p);
                let c = get_color()?;
                stream_logs(o.namespace.clone(), p.clone(), o.tail_length, c).await?;
            }
        }
        Err(error) => println!("error: {}", error),
    }

    Ok(())
}

async fn get_pods(namespace: &str) -> Result<Vec<Pod>> {
    let mut client_config = Config::infer().await?;
    // client_config.timeout = std::time::Duration::from_secs(60 * 60 * 24);
    client_config.timeout = None;
    let client = Client::new(client_config);

    let pods: Api<Pod> = Api::namespaced(client, namespace);
    let mut lp = ListParams::default();
    lp.timeout = None;

    Ok(pods.list(&lp).await?.into_iter().collect())
}

async fn list_pods(namespace: &str) -> Result<HashMap<usize, String>> {
    std::env::set_var("RUST_LOG", "info");
    env_logger::init();

    let pods = get_pods(namespace).await?;
    let mut pod_map = HashMap::new();
    for (i, p) in pods.iter().enumerate() {
        println!("\t{}: {}", i, Meta::name(p));
        pod_map.insert(i, Meta::name(p));
    }

    Ok(pod_map)
}

async fn collect_pods(namespace: &str, pattern: &str) -> Result<Vec<String>> {
    let mut client_config = Config::infer().await?;
    // client_config.timeout = std::time::Duration::from_secs(60 * 60 * 24);
    client_config.timeout = None;
    let client = Client::new(client_config);

    let pods: Api<Pod> = Api::namespaced(client, namespace);
    let mut lp = ListParams::default();
    lp.timeout = None;
    let mut matching_pods = vec![];
    let re: Regex = Regex::new(pattern).unwrap();
    for (_, p) in (pods.list(&lp).await?).into_iter().enumerate() {
        // lazy_static! {
        //     static ref RE: Regex = Regex::new(pattern).unwrap();
        // }
        let name = Meta::name(&p).to_string();
        if re.is_match(&name) {
            matching_pods.push(name);
        }
    }

    Ok(matching_pods)
}

async fn stream_logs(
    namespace: String,
    pod_name: String,
    tail_lines: i64,
    c: color::Rgb,
) -> Result<()> {
    let mut client_config = Config::infer().await?;
    // client_config.timeout = std::time::Duration::from_secs(60 * 60 * 24);
    client_config.timeout = None;
    let client = Client::new(client_config);

    let pods: Api<Pod> = Api::namespaced(client, &namespace);
    let mut lp = LogParams::default();
    lp.follow = true;
    lp.pretty = true;
    lp.tail_lines = Some(tail_lines);
    let mut logs = pods.log_stream(&pod_name, &lp).await?.boxed();
    println!("{}", color::Fg(color::Reset));
    while let Some(line) = logs.try_next().await? {
        let line_str = String::from_utf8((&line).to_vec())?;
        if line_str.contains("ERROR") || line_str.contains("error") {
            print!("{}{}  ", color::Fg(c), pod_name);
            println!(
                "{}{}{}{}",
                color::Fg(color::Red),
                style::Bold,
                line_str,
                style::Reset
            );
        } else {
            print!("{}{}  ", color::Fg(c), pod_name);

            println!("{}{}", color::Fg(color::Reset), line_str);
        }
    }

    Ok(())
}
