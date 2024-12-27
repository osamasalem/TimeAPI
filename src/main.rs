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

const DEFAULT_LOG_LEVEL: &str = "INFO";
const DEFAULT_IP_ADDRESS: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 3000;
const DEFAULT_NTP_SERVER: &str = "time.google.com:123";

struct TimeCache {
    last_ntp: DateTime<Utc>,
    last_updated: SystemTime,
}

struct AppContext {
    ntp_server: String,
    cache_timeout: u64,
    time_cache: RwLock<Option<TimeCache>>,
}

const DEFAULT_CACHE_DURATION: u64 = 5 * 60;

impl AppContext {
    pub fn new(time_out: u64, ntp_server: String) -> Self {
        Self {
            ntp_server,
            cache_timeout: time_out,
            time_cache: RwLock::new(None),
        }
    }

    pub async fn get_time(&self) -> DateTime<Utc> {
        self.get_time_internal()
            .await
            .unwrap_or(self.update_and_return_new_time().await)
    }

    async fn get_time_internal(&self) -> Option<DateTime<Utc>> {
        let lock = self.time_cache.read().await;

        if let Some(time) = lock.deref() {
            let duration: Result<i128, _> = SystemTime::now()
                .duration_since(time.last_updated)
                .map(|x| x.as_secs().into());
            if duration
                .as_ref()
                .is_ok_and(|dur| (..self.cache_timeout.into()).contains(dur))
            {
                return time
                    .last_ntp
                    .checked_add_signed(TimeDelta::seconds(duration.unwrap() as i64));
            }
        }

        None
    }

    fn request_ntp_async<A>(addr: A) -> JoinHandle<ntp::errors::Result<ntp::packet::Packet>>
    where
        A: ToSocketAddrs + Send + 'static,
    {
        tokio::spawn(async move { ntp::request(addr) })
    }

    async fn get_time_from_ntp(&self) -> Result<DateTime<Utc>, Cow<'static, str>> {
        let address = self.ntp_server.clone();
        let response = Self::request_ntp_async(address)
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

    async fn update_and_return_new_time(&self) -> DateTime<Utc> {
        info!("Update from ntp server");

        let mut lock = self.time_cache.write().await;

        if let Some(ref mut time) = lock.deref_mut() {
            let duration: Result<i128, _> = SystemTime::now()
                .duration_since(time.last_updated)
                .map(|x| x.as_secs().into());

            if duration
                .as_ref()
                .is_ok_and(|dur| (self.cache_timeout.into()..).contains(dur))
            {
                if let Ok(val) = self.get_time_from_ntp().await {
                    time.last_ntp = val;
                    time.last_updated = SystemTime::now();
                } else {
                    return time
                        .last_ntp
                        .checked_add_signed(TimeDelta::seconds(duration.unwrap() as i64))
                        .unwrap_or(chrono::Utc::now());
                }
            }

            time.last_ntp
        } else {
            let time = lock.deref_mut();

            self.get_time_from_ntp()
                .await
                .inspect(|val| {
                    *time = Some(TimeCache {
                        last_ntp: *val,
                        last_updated: SystemTime::now(),
                    });
                })
                .inspect_err(|_| error!("Error Get time from NTP"))
                .unwrap_or(chrono::Utc::now())
        }
    }
}

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

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let _ = dotenv();

    let loglevel = env::var("LOG_LEVEL").unwrap_or(DEFAULT_LOG_LEVEL.to_owned());
    let address = env::var("IP").unwrap_or(DEFAULT_IP_ADDRESS.to_owned());
    let port = env::var("PORT")
        .map(|var| var.parse::<u16>().unwrap_or(DEFAULT_PORT))
        .unwrap_or(DEFAULT_PORT);

    let ntp_server = env::var("NTP_SERVER").unwrap_or(DEFAULT_NTP_SERVER.to_owned());

    let cache_timeout = env::var("CACHE_TIMEOUT")
        .map(|var| var.parse::<u64>().unwrap_or(DEFAULT_CACHE_DURATION))
        .unwrap_or(DEFAULT_CACHE_DURATION);

    let _ = TermLogger::init(
        LevelFilter::from_str(&loglevel).unwrap_or(LevelFilter::Info),
        Config::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    );

    let app_state = web::Data::new(AppContext::new(cache_timeout, ntp_server));

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
