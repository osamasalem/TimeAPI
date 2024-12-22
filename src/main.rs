mod models;
mod utils;

use actix_web::{error, get, web, App, HttpResponse, HttpServer, Responder, Result};
use chrono::{DateTime, Months, TimeDelta, TimeZone, Utc};
use chrono_tz::Tz;
use dotenv::dotenv;
use log::{error, info, LevelFilter};
use simplelog::{ColorChoice, Config, TermLogger, TerminalMode};
use std::borrow::Cow;
use std::net::ToSocketAddrs;
use std::ops::{Deref, DerefMut};
use std::time::SystemTime;
use std::{env, str::FromStr};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

struct TimeCache {
    last_ntp: DateTime<Utc>,
    last_updated: SystemTime,
}

struct AppContext {
    time_cache: RwLock<Option<TimeCache>>,
}

const CACHE_DURATION: u64 = 5 * 60;

impl AppContext {
    async fn get_time(&self) -> Result<DateTime<Utc>, Cow<'static, str>> {
        if let Some(val) = self.get_time_internal().await {
            Ok(val)
        } else {
            self.update_and_return_new_time().await
        }
    }

    async fn get_time_internal(&self) -> Option<DateTime<Utc>> {
        let lock = self.time_cache.read().await;

        if let Some(time) = lock.deref() {
            if let Ok(duration @ ..CACHE_DURATION) = SystemTime::now()
                .duration_since(time.last_updated)
                .map(|x| x.as_secs())
            {
                return time
                    .last_ntp
                    .checked_add_signed(TimeDelta::seconds(duration.try_into().ok()?));
            }
        }

        None
    }

    async fn update_and_return_new_time(&self) -> Result<DateTime<Utc>, Cow<'static, str>> {
        info!("Update from ntp server");

        let mut lock = self.time_cache.write().await;

        if let Some(ref mut time) = *lock {
            if let Ok(CACHE_DURATION..) = SystemTime::now()
                .duration_since(time.last_updated)
                .map(|x| x.as_secs())
            {
                time.last_ntp = get_time_from_ntp().await?;
                time.last_updated = SystemTime::now();
            }

            Ok(time.last_ntp)
        } else {
            let time = get_time_from_ntp().await?;

            let rererence = lock.deref_mut();
            *rererence = Some(TimeCache {
                last_ntp: time,
                last_updated: SystemTime::now(),
            });
            Ok(time)
        }
    }
}
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
async fn now(app: web::Data<AppContext>) -> Result<impl Responder> {
    let time = app
        .get_time()
        .await
        .inspect_err(|err| error!("Cannot get time from NTP : {err}"))
        .map_err(|_| error::ErrorInternalServerError("Cannot get time from NTP"))?;
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

    let time = app
        .get_time()
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

    let app_state = web::Data::new(AppContext {
        time_cache: RwLock::new(None),
    });

    info!("Listening on {address}:{port}");
    HttpServer::new(move || {
        App::new()
            .app_data(app_state.clone())
            .service(now)
            .service(health)
            .service(now_with_tz)
    })
    .bind((address, port))?
    .run()
    .await
}
