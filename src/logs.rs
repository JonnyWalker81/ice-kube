use futures::{StreamExt, TryStreamExt};
use std::{collections::HashMap, io};

use anyhow::Result;
use crossterm::{
    event, execute,
    style::{
        Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
    },
    Result as CrossResult,
};
use futures::future::join_all;
use k8s_openapi::api::core::v1::Pod;
use kube::{
    api::{ListParams, LogParams, Meta},
    config::KubeConfigOptions,
    Api, Client, Config,
};
use lazy_static::lazy_static;
use log::{debug, error, info, log_enabled, Level};
use regex::Regex;
use std::io::{stdout, Write};

use tokio::task;

use crate::{
    util::{get_pods, OptionEx},
    LogsOpts,
};

lazy_static! {
    static ref COLORS: Vec<Color> = vec![
        Color::Rgb { r: 0, g: 255, b: 0 },
        Color::Rgb {
            r: 128,
            g: 128,
            b: 0
        },
        Color::Rgb {
            r: 0,
            g: 255,
            b: 255
        },
        Color::Rgb {
            r: 255,
            g: 192,
            b: 203
        },
        Color::Rgb {
            r: 245,
            g: 120,
            b: 250
        },
        Color::Rgb {
            r: 221,
            g: 160,
            b: 221
        },
        Color::Rgb {
            r: 154,
            g: 205,
            b: 50
        },
        Color::Rgb {
            r: 230,
            g: 230,
            b: 120
        },
    ];
}

pub fn get_color() -> Result<Color> {
    use rand::Rng;

    let mut rng = rand::thread_rng();
    let index = rng.gen_range(0.0, COLORS.len() as f32) as usize;
    let rgb = COLORS[index];

    Ok(rgb)
}

pub async fn follow_logs(o: &LogsOpts, p: &str) -> Result<()> {
    let pods = collect_pods(&o.namespace, &p).await?;
    println!("Pods: {:?}", pods);
    let mut tasks = vec![];
    for name in pods {
        let c = get_color()?;
        let n = o.namespace.clone();
        let pn = name.clone();

        let h = o.highlight.to_str();

        let t = task::spawn(stream_logs(n, pn, o.tail_length, c, h, o.filter));
        tasks.push(t);
    }

    join_all(tasks).await;

    Ok(())
}

pub async fn select_pod(o: &LogsOpts) -> Result<()> {
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

                let h = o.highlight.to_str();

                stream_logs(
                    o.namespace.clone(),
                    p.clone(),
                    o.tail_length,
                    c,
                    h,
                    o.filter,
                )
                .await?;
            }
        }
        Err(error) => println!("error: {}", error),
    }

    Ok(())
}

async fn list_pods(namespace: &str) -> Result<HashMap<usize, String>> {
    // std::env::set_var("RUST_LOG", "info");
    // env_logger::init();

    let pods = get_pods(namespace).await?;
    let mut pod_map = HashMap::new();
    for (i, p) in pods.iter().enumerate() {
        println!("\t{}: {}", i, Meta::name(p));
        pod_map.insert(i, Meta::name(p));
    }

    Ok(pod_map)
}

async fn collect_pods(namespace: &str, pattern: &str) -> Result<Vec<String>> {
    let mut client_config = match Config::infer().await {
        Ok(c) => c,
        Err(_) => Config::from_kubeconfig(&KubeConfigOptions::default()).await?,
    };

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

pub async fn stream_logs(
    namespace: String,
    pod_name: String,
    tail_lines: i64,
    c: Color,
    highlight: String,
    filter: bool,
) -> Result<()> {
    let mut client_config = match Config::infer().await {
        Ok(c) => c,
        Err(_) => Config::from_kubeconfig(&KubeConfigOptions::default()).await?,
    };

    // client_config.timeout = std::time::Duration::from_secs(60 * 60 * 24);
    client_config.timeout = None;
    let client = Client::new(client_config);

    let pods: Api<Pod> = Api::namespaced(client, &namespace);
    let mut lp = LogParams::default();
    lp.follow = true;
    lp.pretty = true;
    lp.tail_lines = Some(tail_lines);
    let mut logs = pods.log_stream(&pod_name, &lp).await?.boxed();
    let re: Regex = Regex::new(&highlight).unwrap();
    execute!(stdout(), ResetColor)?;
    while let Some(line) = logs.try_next().await? {
        let line_str = String::from_utf8((&line).to_vec())?;
        if filter {
            if !highlight.is_empty() && re.is_match(&line_str) {
                execute!(
                    stdout(),
                    SetForegroundColor(Color::Yellow),
                    SetAttribute(Attribute::Bold),
                    Print(line_str),
                    ResetColor
                )?;
                println!();
            }
        } else {
            execute!(
                stdout(),
                SetForegroundColor(c),
                Print(&pod_name),
                Print(" ")
            )?;
            if line_str.contains("ERROR")
                || line_str.contains("error")
                || line_str.contains("Error")
            {
                execute!(
                    stdout(),
                    SetForegroundColor(Color::Red),
                    SetAttribute(Attribute::Bold),
                    Print(line_str),
                    ResetColor
                )?;
            } else if !highlight.is_empty() && re.is_match(&line_str) {
                execute!(
                    stdout(),
                    SetForegroundColor(Color::Yellow),
                    SetAttribute(Attribute::Bold),
                    Print(line_str),
                    ResetColor
                )?;
            } else {
                execute!(stdout(), ResetColor, Print(line_str))?;
            }
            println!();
        }
    }

    Ok(())
}
