mod models;
mod utils;

use actix_web::{error, get, web, App, HttpResponse, HttpServer, Responder, Result};
use chrono::{DateTime, Months, TimeZone, Utc};
use chrono_tz::Tz;
use dotenv::dotenv;
use log::{error, info, LevelFilter};
use simplelog::{ColorChoice, Config, TermLogger, TerminalMode};
use std::borrow::Cow;
use std::net::ToSocketAddrs;
use std::{env, str::FromStr};
use tokio::task::JoinHandle;

fn request_ntp_async<A>(addr: A) -> JoinHandle<ntp::errors::Result<ntp::packet::Packet>>
where
    A: ToSocketAddrs + Send + 'static,
{
    tokio::spawn(async move { ntp::request(addr) })
}

async fn get_time_from_ntp() -> Result<DateTime<Utc>, Cow<'static, str>> {
    let address = "time.google.com:123";
    let response = request_ntp_async(address)
        .await
        .map_err(|err| format!("failed to join async taks :{err}"))?
        .map_err(|err| format!("connection to ntp failed : {err}"))?;

    let ntp_time = response.ref_time;

    info!("from ntp : {sec}", sec = ntp_time.sec);

    let time = chrono::Utc
        .timestamp_opt(ntp_time.sec as i64, 0)
        .single()
        .ok_or(format!("Error to get single time from {}", ntp_time.sec))?
        .checked_sub_months(Months::new(70 * 12))
        .ok_or(format!("Error to adjust time from {}", ntp_time.sec))?;
    Ok(time)
}

#[get("/health")]
async fn health() -> impl Responder {
    HttpResponse::Ok()
}

#[get("/now")]
async fn now() -> Result<impl Responder> {
    let time = get_time_from_ntp()
        .await
        .inspect_err(|err| error!("Cannot get time from NTP : {err}"))
        .map_err(|_| error::ErrorInternalServerError("Cannot get time from NTP"))?;
    info!("/now: {:?}", time);
    Ok(web::Json(models::Time::from(time)))
}

#[get("/now/{continent}/{region}")]
async fn now_with_tz(args: web::Path<models::TimeZone>) -> Result<impl Responder> {
    let timezone: Tz = format!(
        "{cont}/{region}",
        cont = utils::to_camel_case(&args.continent),
        region = utils::to_camel_case(&args.region)
    )
    .parse()
    .map_err(|_| error::ErrorBadRequest("Invalid Time zone"))?;

    let time = get_time_from_ntp()
        .await
        .inspect_err(|err| error!("Cannot get time from NTP : {err}"))
        .map_err(|_| error::ErrorInternalServerError("Cannot get time from NTP"))?;

    let time = timezone.from_utc_datetime(&time.naive_utc());
    info!("now with tz: {:?}", time);
    Ok(web::Json(models::Time::from(time)))
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let _ = dotenv();

    let loglevel = env::var("TIMEAPI_LOG_LEVEL").unwrap_or("INFO".to_owned());
    let address = env::var("TIMEAPI_ADDRESS").unwrap_or("127.0.0.1".to_owned());
    let port = env::var("TIMEAPI_PORT")
        .map(|var| var.parse::<u16>().unwrap_or(3000))
        .unwrap_or(3000);

    let _ = TermLogger::init(
        LevelFilter::from_str(&loglevel).unwrap_or(LevelFilter::Info),
        Config::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    );

    info!("Listening on {address}:{port}");
    HttpServer::new(|| App::new().service(now).service(health).service(now_with_tz))
        .bind((address, port))?
        .run()
        .await
}
