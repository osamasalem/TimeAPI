mod app_context;
mod models;
mod utils;

use actix_cors::Cors;
use actix_web::{error, get, web, App, HttpResponse, HttpServer, Responder, Result};
use app_context::AppContext;
use chrono::TimeZone;
use chrono_tz::Tz;
use log::{info, LevelFilter};
use simplelog::{ColorChoice, Config, TermLogger, TerminalMode};
use std::fmt::Debug;
use std::{env, str::FromStr};

const DEFAULT_IP_ADDRESS: &str = "127.0.0.1";
const DEFAULT_NTP_SERVER: &str = "time.google.com:123";
const DEFAULT_PORT: u16 = 3000;
const DEFAULT_CACHE_DURATION: u64 = 5 * 60; // sec
const DEFAULT_CORS_ORIGIN: &str = "127.0.0.1";

#[get("/health")]
async fn health() -> impl Responder {
    HttpResponse::Ok()
}

#[get("/now")]
async fn now(app: web::Data<AppContext>) -> Result<impl Responder> {
    let time = app.get_time().await;
    info!("/now: {:?}", time);
    Ok(web::Json(models::Time::from(time)))
}

#[get("/now/{continent}/{region}")]
async fn now_with_tz(
    args: web::Path<models::TimeZone>,
    app: web::Data<AppContext>,
) -> Result<impl Responder> {
    let timezone: Tz = format!(
        "{cont}/{region}",
        cont = utils::to_camel_case(&args.continent),
        region = utils::to_camel_case(&args.region)
    )
    .parse()
    .map_err(|_| error::ErrorBadRequest("Invalid Time zone"))?;

    let time = app.get_time().await;

    let time = timezone.from_utc_datetime(&time.naive_utc());
    info!("now with tz: {:?}", time);
    Ok(web::Json(models::Time::from(time)))
}

#[inline]
fn parse_env<T>(name: &str, default: T) -> T
where
    T: FromStr + Debug,
{
    env::var(name)
        .ok()
        .and_then(|val| {
            println!("Get the {name} value from env:{val:?}");
            val.parse::<T>().ok()
        })
        .unwrap_or_else(|| {
            println!("Get the default value fro {name} :{default:?}");
            default
        })
}

#[actix_web::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    let _ = dotenvy::dotenv()?;

    let loglevel = parse_env("LOG_LEVEL", LevelFilter::Info);
    let address = parse_env("IP", DEFAULT_IP_ADDRESS.to_owned());
    let port = parse_env("PORT", DEFAULT_PORT);
    let ntp_server = parse_env("NTP_SERVER", DEFAULT_NTP_SERVER.to_owned());
    let cache_timeout = parse_env("CACHE_TIMEOUT", DEFAULT_CACHE_DURATION);
    let cors_origin = parse_env("CORS_ORIGIN", DEFAULT_CORS_ORIGIN.to_owned());

    TermLogger::init(
        loglevel,
        Config::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )?;

    let app_state = web::Data::new(AppContext::new(cache_timeout, ntp_server));

    info!("Listening on {address}:{port}");

    HttpServer::new(move || {
        let cors = Cors::default()
            .allowed_origin(cors_origin.clone().as_str())
            .allowed_methods(vec!["GET"])
            .max_age(3600);

        App::new()
            .app_data(app_state.clone())
            .wrap(cors)
            .service(now)
            .service(health)
            .service(now_with_tz)
    })
    .bind((address, port))?
    .run()
    .await?;

    Ok(())
}
