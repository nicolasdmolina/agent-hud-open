//! What one round of `identify` over a far machine's homes adds up to: its
//! gateways, and the profiles each serves.
use super::{
    super::connection::{identify, Identified, Instance},
    Answered, FarHome, Homes,
};
use std::collections::BTreeMap;

/// What one home's control socket did when it was asked. `Q` is why a home
/// gave no answer, as whoever asked knows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Asked<Q> {
    /// It answered, and this is what the answer makes the connection.
    Answered(Identified),
    Quiet(Q),
}

/// One gateway process on the far machine: every home whose socket
/// answered as the same instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundGateway {
    pub instance_id: String,
    /// The homes whose socket answered as this instance, by path, each
    /// with the instance as that home's answer gives it: its `profile_id`
    /// is the one a ticket asked through that home names.
    pub sockets: Vec<(FarHome, Instance)>,
    /// The profiles it says it serves, among the homes that were asked,
    /// by path. Far paths, compared with far paths.
    pub served: Vec<FarHome>,
}

/// The gateways of one far machine, as one round of `identify` found them.
/// The same whichever home was asked first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture<Q> {
    /// The Hermes root home there, canonical.
    pub root: String,
    /// By the first home each serves, then by instance.
    pub gateways: Vec<FoundGateway>,
    /// Every home that was asked and what came of it, in the homes' order.
    pub asked: Vec<(FarHome, Asked<Q>)>,
    /// Homes that two instances both say they serve: a restart overlapping.
    /// No card is issued for one until a later round agrees.
    pub contested: Vec<FarHome>,
}

/// The homes an `identify` answer says its gateway serves, as it wrote them.
fn served_homes(answer: &Answered) -> Vec<&str> {
    let answer = answer.value();
    let told = match answer.get("result") {
        Some(result) if result.is_object() => result,
        _ => answer,
    };
    told["served_profiles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry["home"].as_str())
        .collect()
}

/// Groups the answers by `instance_id` into gateways. `answers` holds each
/// home that was asked once, in any order. A served home is counted only
/// where it is, as a far path, one of the homes that were asked.
pub fn picture<Q>(homes: &Homes, answers: Vec<(FarHome, Result<Answered, Q>)>) -> Picture<Q> {
    let mut found: BTreeMap<String, FoundGateway> = BTreeMap::new();
    let mut asked = Vec::new();
    for (home, answer) in answers {
        let answered = match answer {
            Err(quiet) => {
                asked.push((home, Asked::Quiet(quiet)));
                continue;
            }
            Ok(answered) => answered,
        };
        let identified = identify(answered.value(), &home.path);
        if let Identified::Ready(instance) = &identified {
            let gateway = found
                .entry(instance.id.clone())
                .or_insert_with(|| FoundGateway {
                    instance_id: instance.id.clone(),
                    sockets: Vec::new(),
                    served: Vec::new(),
                });
            gateway.sockets.push((home.clone(), instance.clone()));
            for served in served_homes(&answered) {
                let known = homes.homes.iter().find(|known| known.path == served);
                if let Some(known) = known.filter(|known| !gateway.served.contains(known)) {
                    gateway.served.push(known.clone());
                }
            }
        }
        asked.push((home, Asked::Answered(identified)));
    }
    let place = |home: &FarHome| homes.homes.iter().position(|known| known == home);
    asked.sort_by_key(|(home, _)| place(home));
    let mut gateways: Vec<FoundGateway> = found.into_values().collect();
    for gateway in &mut gateways {
        gateway.sockets.sort_by(|a, b| a.0.path.cmp(&b.0.path));
        gateway.served.sort_by(|a, b| a.path.cmp(&b.path));
    }
    gateways.sort_by(|a, b| {
        let first = |gateway: &FoundGateway| gateway.served.first().map(|home| home.path.clone());
        (first(a), &a.instance_id).cmp(&(first(b), &b.instance_id))
    });
    let claims = |home: &FarHome| {
        gateways
            .iter()
            .filter(|gateway| gateway.served.contains(home))
            .count()
    };
    let contested = homes
        .homes
        .iter()
        .filter(|home| claims(home) > 1)
        .cloned()
        .collect();
    Picture {
        root: homes.root.clone(),
        gateways,
        asked,
        contested,
    }
}
