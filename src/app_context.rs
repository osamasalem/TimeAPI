use std::{
    borrow::Cow,
    ops::{Deref, DerefMut},
    time::SystemTime,
};

use chrono::{DateTime, Months, TimeDelta, TimeZone, Utc};
use log::{error, info, trace};
use tokio::sync::RwLock;

struct TimeCache {
    last_ntp: DateTime<Utc>,
    last_updated: SystemTime,
}

pub struct AppContext {
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
        self.fast_get_time_from_cache()
            .await
            .unwrap_or_else(|| async { self.update_and_return_new_time().await });
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

#[tokio::test]
async fn test_fast_get_time_from_cache_fail() {
    let app = AppContext::new(5, "aaa".to_owned());
    assert_eq!(app.fast_get_time_from_cache().await, None);
}

#[tokio::test]
async fn test_fast_get_time_from_cache_fail_cache() {
    let app = AppContext::new(1, "aaa".to_owned());
    app.update_and_return_new_time().await;
    assert_eq!(app.fast_get_time_from_cache().await, None);
}

#[tokio::test]
async fn test_fast_get_time_from_cache_success() {
    let app = AppContext::new(5, "time.google.com:123".to_owned());
    app.update_and_return_new_time().await;
    assert!(app.fast_get_time_from_cache().await.is_some());
}

#[tokio::test]
async fn test_fast_get_time_from_cache_timeout() {
    let app = AppContext::new(1, "time.google.com:123".to_owned());
    app.update_and_return_new_time().await;
    let _ = tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    assert!(app.fast_get_time_from_cache().await.is_none());
}
