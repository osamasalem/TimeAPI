mod models;
mod utils;

use actix_cors::Cors;
use actix_web::{error, get, web, App, HttpResponse, HttpServer, Responder, Result};
use chrono::{DateTime, Months, TimeDelta, TimeZone, Utc};
use chrono_tz::Tz;
use dotenv::dotenv;
use log::{error, info, trace, LevelFilter};
use simplelog::{ColorChoice, Config, TermLogger, TerminalMode};
use std::borrow::Cow;
use std::fmt::Debug;
use std::ops::{Deref, DerefMut};
use std::time::SystemTime;
use std::{env, str::FromStr};
use tokio::sync::RwLock;

const DEFAULT_IP_ADDRESS: &str = "127.0.0.1";
const DEFAULT_NTP_SERVER: &str = "time.google.com:123";
const DEFAULT_PORT: u16 = 3000;
const DEFAULT_CACHE_DURATION: u64 = 5 * 60; // sec
const DEFAULT_CORS_ORIGIN: &str = "127.0.0.1";

struct TimeCache {
    last_ntp: DateTime<Utc>,
    last_updated: SystemTime,
}

struct AppContext {
    ntp_server: String,
    cache_timeout: u64,
    time_cache: RwLock<Option<TimeCache>>,
}

impl AppContext {
    pub fn new(time_out: u64, ntp_server: String) -> Self {
          
        Self {
            ntp_server,
            cache_timeout: time_out,
            time_cache: RwLock::new(None),
        }
    }

    pub async fn get_time(&self) -> DateTime<Utc> {
        match self.fast_get_time_from_cache().await {
            Some(val) => val,
            _ => self.update_and_return_new_time().await,
        }
    }

    async fn fast_get_time_from_cache(&self) -> Option<DateTime<Utc>> {
        trace!("Read time from cache");

        let lock = self.time_cache.read().await;

        if let Some(time) = lock.deref() {
            let duration: Result<i128, _> = SystemTime::now()
                .duration_since(time.last_updated)
                .map(|x| x.as_secs().into());
            if duration
                .as_ref()
                .is_ok_and(|dur| (..self.cache_timeout.into()).contains(dur))
            {
                trace!("cache is not expired {duration:?}");

                let ret = time
                    .last_ntp
                    .checked_add_signed(TimeDelta::seconds(duration.unwrap() as i64));

                trace!("cache is not expired #2 {ret:?}");
                return ret;
            }
        }

        None
    }

    async fn get_time_from_ntp(&self) -> Result<DateTime<Utc>, Cow<'static, str>> {
        let address = self.ntp_server.clone();
        let response =
            ntp::request(address).map_err(|err| format!("connection to ntp failed : {err}"))?;

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
            trace!("Use the cache");
            let duration: Result<i128, _> = SystemTime::now()
                .duration_since(time.last_updated)
                .map(|x| x.as_secs().into());

            if duration
                .as_ref()
                .is_ok_and(|dur| (self.cache_timeout.into()..).contains(dur))
            {
                trace!("cache is time out");
                if let Ok(val) = self.get_time_from_ntp().await {
                    trace!("Update the cache");

                    time.last_ntp = val;
                    time.last_updated = SystemTime::now();
                } else {
                    trace!("Fallback to the value we have");
                    return time
                        .last_ntp
                        .checked_add_signed(TimeDelta::seconds(duration.unwrap() as i64))
                        .unwrap_or(chrono::Utc::now());
                }
            }

            time.last_ntp
        } else {
            trace!("Instentiate the new cache");
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

    let _ = dotenv();

    let loglevel = parse_env("LOG_LEVEL", LevelFilter::Info);
    let address = parse_env("IP", DEFAULT_IP_ADDRESS.to_owned());
    let port = parse_env("PORT", DEFAULT_PORT);
    let ntp_server = parse_env("NTP_SERVER", DEFAULT_NTP_SERVER.to_owned());
    let cache_timeout = parse_env("CACHE_TIMEOUT", DEFAULT_CACHE_DURATION);
    let cors_origin = parse_env("CORS_ORIGIN", DEFAULT_CORS_ORIGIN.to_owned());

    let _ = TermLogger::init(
        loglevel,
        Config::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    );

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
    .await
}
