use std::collections::HashSet;
use std::fs::read_to_string;
use std::path::PathBuf;
use std::{collections::HashMap, time::Duration, fs::File};
use std::io::Write;

use log::info;
use scraper::{Element, ElementRef, Html, Selector};
use vlrx::{Agent, Map, Match, Player, Team};

use clap::{Parser, Subcommand, builder::ValueParser};

fn validator_url() -> ValueParser {
    ValueParser::from(move |s: &str| -> std::result::Result<String, String> {
        let reg = regex::Regex::new(r#"https?:\/\/(www\.)?vlr\.gg\b([-a-zA-Z0-9()@:%_\+.~#?&//=]*)"#).unwrap();
        match reg.is_match(s) {
            true => {
                if s.ends_with('/') {
                    Err("Don't include / at the end of the url".to_string())
                } else {
                    Ok(s.to_owned())
                }
            }
            false => Err("Invalid vlr.gg url".to_string()),
        }
    })
}

#[derive(Parser)]
#[command(name = "vctd")]
#[command(about = "Data scraper and analyzer for VCT (from vlr.gg)", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands
}

#[derive(Subcommand)]
enum Commands {
    #[command(arg_required_else_help = true, about = "Scrape vlr.gg event")]
    Scrape {
        #[arg(value_parser = validator_url(), help = "VLR.gg URL of the event to parse")]
        event_url: String,
        #[arg(short, help = "Output file")]
        output: PathBuf,
    },
    #[command(arg_required_else_help = true, about = "Analyze scraped data")]
    Analyze {
        #[arg(required = true, help = "JSON data file path")]
        data_path: PathBuf,
        #[command(subcommand)]
        subcmd: AnalyzeCommands,
    }
}
#[derive(Subcommand)]
enum AnalyzeCommands {
    #[command(arg_required_else_help = true, about = "analyze maps")]
    Maps {
        #[arg(required_unless_present = "list")]
        map_name: Option<String>,
        #[arg(short, long, help = "To analyze meta")]
        meta: bool,
        #[arg(short, long, help = "List all maps in dataset", exclusive = true)]
        list: bool,
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let args = Cli::parse();
    match args.command {
        Commands::Analyze { data_path, subcmd } => {
            let contents = read_to_string(data_path)?;
            let matches = serde_json::from_str::<Vec<Match>>(&contents)?;
            match subcmd {
                AnalyzeCommands::Maps { map_name, meta, list } => {
                    if list {
                        let maps = get_maps(&matches);
                        println!("{:#?}", maps);
                    } else {
                        if meta {
                            let map_meta = analyze_meta(&matches, &map_name.expect("to be present w/o list"));
                            println!("{:#?}", map_meta);
                        }
                    }
                }
            }
            Ok(())
        }
        Commands::Scrape { event_url, output } => {
            scrape_url(event_url, output).await
        }
    }
}

fn get_maps(matches: &Vec<Match>) -> HashSet<Map> {
    let mut res = HashSet::new();
    for m in matches {
        res.insert(m.map.clone());
    }
    res
}

fn analyze_meta<'a>(matches: &'a Vec<Match>, map: &str) -> Vec<(Vec<&'a Agent>, f64)> {
    let mut map_meta: HashMap<(&Map, Vec<&Agent>), usize> = HashMap::new();
    for m in matches {
        let mut comp1 = m.agents.iter().filter_map(|(k, v)| {
            if m.team_won.players.contains(&k) {
                Some(v)
            } else { None }
        }).collect::<Vec<&Agent>>();
        comp1.sort();
        let mut comp2 = m.agents.iter().filter_map(|(k, v)| {
            if m.team_lost.players.contains(&k) {
                Some(v)
            } else { None }
        }).collect::<Vec<&Agent>>();
        comp2.sort();
        map_meta.entry((&m.map, comp1)).and_modify(|e| *e += 1).or_insert(1);
        map_meta.entry((&m.map, comp2)).and_modify(|e| *e += 1).or_insert(1);
    }

    let spec_map_matches_cnt = matches.iter().filter(|x| x.map.name == map).count() * 2;
    let mut spec_map_meta = map_meta.into_iter().filter(|((&ref m, _), _)| m.name == map)
        .map(|((_, a), c)| (a, c as f64 / spec_map_matches_cnt as f64 ))
        .collect::<Vec<_>>();
    spec_map_meta.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    return spec_map_meta;
}

async fn scrape_url(event_url: String, output: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let resp_text =
        reqwest::get(event_url)
            .await?
            .text()
            .await?;

    info!("Fetched initial page...");
    tokio::time::sleep(Duration::from_secs(1)).await;
    let doc = Html::parse_document(&resp_text);
    let mut matches = parse_event(&doc).await?;

    info!("Found matches from initial event");
    let subnav_sel = Selector::parse(".wf-subnav-item:not(.mod-active)")?;
    let event_pages = doc
        .select(&subnav_sel)
        .filter_map(|x| {
            let href = x.value().attr("href").expect("subnav item is link");
            if href.contains("showmatch") {
                None
            } else {
                Some(href)
            }
        })
        .collect::<Vec<&str>>();

    info!("Going to fetch following pages: {:?}", event_pages);
    for e in event_pages {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let resp_text = reqwest::get(format!("https://vlr.gg{}", e))
            .await?
            .text()
            .await?;
        let doc = Html::parse_document(&resp_text);

        info!("Parsing event {}", e);
        matches.extend(parse_event(&doc).await?);
    }
    let serialized = serde_json::to_string(&matches)?;
    let mut f = File::create(output)?;
    f.write_all(serialized.as_bytes())?;
    Ok(())
}

async fn parse_event(doc: &Html) -> Result<Vec<Match>, Box<dyn std::error::Error>> {
    let mut res_matches = Vec::new();
    let sel = Selector::parse("a.bracket-item")?;
    let bracket_items = doc.select(&sel);
    for element in bracket_items {
        let series_url = element.value().attr("href").expect("bracket item to have link");

        info!("Parsing series {}", series_url);
        let series_page = reqwest::get(format!("https://www.vlr.gg{}", series_url))
            .await?
            .text()
            .await?;
        res_matches.extend(parse_matches(&series_page)?);
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Ok(res_matches)
}
fn parse_matches(page_cont: &String) -> Result<Vec<Match>, Box<dyn std::error::Error>> {
    let mut res_matches = Vec::new();
    let series_doc = Html::parse_document(page_cont);
    let match_sel = Selector::parse(".vm-stats-game")?;
    let match_data_pages = series_doc
        .select(&match_sel)
        .filter(|x| x.value().attr("data-game-id") != Some("all"));
    for match_data in match_data_pages {
        let team_selector = Selector::parse(".team")?;
        let win_selector = Selector::parse(".score.mod-win")?;
        let score_selector = Selector::parse(".score")?;
        let header_selector = Selector::parse(".vm-stats-game-header")?;
        let map_selector = Selector::parse(".map")?;
        let header = match_data
            .select(&header_selector)
            .next()
            .expect("game tab should have header");
        let map = header
            .select(&map_selector)
            .next()
            .unwrap()
            .first_element_child()
            .unwrap()
            .first_element_child()
            .unwrap()
            .text()
            .next()
            .unwrap()
            .trim();
        let teams = header.select(&team_selector);
        let mut team_won = String::new();
        let mut won_score = 0;
        let mut team_lost = String::new();
        let mut lost_score = 0;
        let mut first_team = String::new();
        let mut second_team = String::new();
        for (i, team) in teams.enumerate() {
            if i == 0 {
                first_team = parse_team_name(&team);
            } else {
                second_team = parse_team_name(&team);
            }
            if let Some(won_score_) = team.select(&win_selector).next() {
                team_won = parse_team_name(&team);
                won_score = won_score_.text().next().unwrap().trim().parse::<u32>()?;
            } else {
                team_lost = parse_team_name(&team);
                lost_score = team
                    .select(&score_selector)
                    .next()
                    .unwrap()
                    .text()
                    .next()
                    .unwrap()
                    .trim()
                    .parse::<u32>()?;
            }
        }

        let table_sel = Selector::parse("table.mod-overview")?;
        let row_sel = Selector::parse("tbody>tr")?;
        let player_name_sel = Selector::parse(".mod-player .text-of")?;
        let agent_sel = Selector::parse(".mod-agent img")?;
        let team_datas = match_data.select(&table_sel);
        let mut agents = HashMap::new();
        let mut team_players: HashMap<String, Vec<String>> = HashMap::new();

        for (i, team_data) in team_datas.enumerate() {
            let players = team_data.select(&row_sel);
            let team = if i == 0 {
                first_team.clone()
            } else {
                second_team.clone()
            };
            for player in players {
                let name = player
                    .select(&player_name_sel)
                    .next()
                    .unwrap()
                    .text()
                    .next()
                    .unwrap()
                    .trim()
                    .to_owned();
                team_players
                    .entry(team.clone())
                    .and_modify(|ps| ps.push(name.clone()))
                    .or_insert(vec![name.clone()]);
                let agent = player
                    .select(&agent_sel)
                    .next()
                    .unwrap()
                    .value()
                    .attr("title")
                    .unwrap()
                    .to_owned();
                agents.insert(name, agent);
            }
        }
        let agents = agents
            .into_iter()
            .map(|(k, v)| (Player { name: k }, Agent { name: v }))
            .collect();
        let mut won_players = vec![];
        let mut lost_players = vec![];
        team_players.into_iter().for_each(|(t, p)| {
            if t == team_won {
                won_players = p.into_iter().map(|name| Player { name }).collect();
            } else if t == team_lost {
                lost_players = p.into_iter().map(|name| Player { name }).collect();
            }
        });
        let won_team_obj = Team {
            name: team_won,
            players: won_players,
        };
        let lost_team_obj = Team {
            name: team_lost,
            players: lost_players,
        };
        res_matches.push(Match {
            agents,
            map: Map {
                name: map.to_owned(),
            },
            team_won: won_team_obj,
            team_lost: lost_team_obj,
            won_score,
            lost_score,
        })
    }
    Ok(res_matches)
}

fn parse_team_name(team_elem: &ElementRef) -> String {
    let team_name_sel = Selector::parse(".team-name").expect("correct selector");
    team_elem
        .select(&team_name_sel)
        .next()
        .unwrap()
        .text()
        .next()
        .unwrap()
        .trim()
        .to_owned()
}
