//! Between Foundation objects and the crate's plain types.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{Encoding, Message};
use objc2_foundation::{
    NSArray, NSDate, NSDictionary, NSError, NSHTTPCookie, NSHTTPCookieDomain, NSHTTPCookieExpires, NSHTTPCookieName,
    NSHTTPCookiePath, NSHTTPCookiePropertyKey, NSHTTPCookieSameSiteLax, NSHTTPCookieSameSitePolicy,
    NSHTTPCookieSameSiteStrict, NSHTTPCookieSecure, NSHTTPCookieValue, NSNull, NSNumber, NSString, NSURL,
    NSURLErrorFailingURLErrorKey,
};
use serde_json::{Map, Number, Value};

use crate::{Cookie, SameSite};

/// A JavaScript result or message body as JSON. Values JSON cannot hold (dates) become numbers
/// or strings.
pub(crate) fn json(object: Option<&AnyObject>) -> Value {
    let Some(object) = object else { return Value::Null };
    if object.downcast_ref::<NSNull>().is_some() {
        return Value::Null;
    }
    if let Some(s) = object.downcast_ref::<NSString>() {
        return Value::String(s.to_string());
    }
    if let Some(n) = object.downcast_ref::<NSNumber>() {
        return match n.encoding() {
            Encoding::Char | Encoding::Bool => Value::Bool(n.as_bool()),
            Encoding::Float | Encoding::Double => {
                let f = n.as_f64();
                if f.fract() == 0. && f.abs() < 9.0e15 {
                    Value::Number(Number::from(f as i64))
                } else {
                    Number::from_f64(f).map(Value::Number).unwrap_or(Value::Null)
                }
            }
            _ => Value::Number(Number::from(n.as_i64())),
        };
    }
    if let Some(a) = object.downcast_ref::<NSArray>() {
        return Value::Array(a.iter().map(|o| json(Some(&o))).collect());
    }
    if let Some(d) = object.downcast_ref::<NSDictionary>() {
        let (keys, values) = d.to_vecs();
        let mut map = Map::new();
        for (k, v) in keys.iter().zip(values.iter()) {
            let key = k.downcast_ref::<NSString>().map(|s| s.to_string()).unwrap_or_else(|| format!("{k:?}"));
            map.insert(key, json(Some(v)));
        }
        return Value::Object(map);
    }
    if let Some(d) = object.downcast_ref::<NSDate>() {
        return Number::from_f64(d.timeIntervalSince1970()).map(Value::Number).unwrap_or(Value::Null);
    }
    Value::String(format!("{object:?}"))
}

pub(crate) fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

pub(crate) fn url(s: &str) -> Option<Retained<NSURL>> {
    NSURL::URLWithString(&ns(s))
}

pub(crate) fn url_string(url: Option<&NSURL>) -> String {
    url.and_then(|u| u.absoluteString()).map(|s| s.to_string()).unwrap_or_default()
}

/// The URL a failed navigation was going to, from the error.
pub(crate) fn failing_url(error: &NSError) -> Option<String> {
    let info = error.userInfo();
    let get = |key: &NSString| info.objectForKey(key);
    get(unsafe { NSURLErrorFailingURLErrorKey }).and_then(|o| o.downcast::<NSURL>().ok()).map(|u| url_string(Some(&u)))
}

pub(crate) fn cookie(c: &NSHTTPCookie) -> Cookie {
    let same_site = match c.sameSitePolicy() {
        Some(p) if &*p == unsafe { NSHTTPCookieSameSiteStrict } => SameSite::Strict,
        Some(p) if &*p == unsafe { NSHTTPCookieSameSiteLax } => SameSite::Lax,
        _ => SameSite::None,
    };
    Cookie {
        name: c.name().to_string(),
        value: c.value().to_string(),
        domain: c.domain().to_string(),
        path: c.path().to_string(),
        secure: c.isSecure(),
        http_only: c.isHTTPOnly(),
        expires: c.expiresDate().map(|d| d.timeIntervalSince1970()),
        same_site,
    }
}

pub(crate) fn ns_cookie(c: &Cookie) -> Option<Retained<NSHTTPCookie>> {
    let mut keys: Vec<&NSHTTPCookiePropertyKey> = Vec::new();
    let mut values: Vec<Retained<AnyObject>> = Vec::new();
    let mut put = |k: &'static NSHTTPCookiePropertyKey, v: Retained<AnyObject>| {
        keys.push(k);
        values.push(v);
    };
    unsafe {
        put(NSHTTPCookieName, ns(&c.name).into());
        put(NSHTTPCookieValue, ns(&c.value).into());
        put(NSHTTPCookieDomain, ns(&c.domain).into());
        put(NSHTTPCookiePath, ns(if c.path.is_empty() { "/" } else { &c.path }).into());
        if c.secure {
            put(NSHTTPCookieSecure, ns("TRUE").into());
        }
        if let Some(expires) = c.expires {
            put(NSHTTPCookieExpires, NSDate::dateWithTimeIntervalSince1970(expires).into());
        }
        match c.same_site {
            SameSite::Lax => put(NSHTTPCookieSameSitePolicy, NSHTTPCookieSameSiteLax.retain().into()),
            SameSite::Strict => put(NSHTTPCookieSameSitePolicy, NSHTTPCookieSameSiteStrict.retain().into()),
            SameSite::None => {}
        }
    }
    let values: Vec<&AnyObject> = values.iter().map(|v| &**v).collect();
    let properties = NSDictionary::from_slices(&keys, &values);
    unsafe { NSHTTPCookie::cookieWithProperties(&properties) }
}
