//! This crate implements a toy payment engine that processes CSV files containing deposits, withdrawals, disputes, chargebacks, and dispute resolutions.

use anyhow::Result;
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt::Debug;

/// The actions that can be performed on an account. (deposit, withdrawal, dispute, resolve, chargeback).
pub mod actions;
/// The client's account.
pub mod client;
/// The database of clients and transactions.
pub mod database;

// we use a newtype pattern to make our code more type-safe and easier to update.
// By using a new struct instead of `type X = Y;` we block ourselves from accidentally performing arithmetic on the id's.
/// The ID of a transaction (deposit or withdrawal).
/// These are globally unique but need not be sequential.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TransactionId(u32);

impl Debug for TransactionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{}", self.0))
    }
}

/// The ID of a client.
/// These are unique but need not be sequential.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ClientId(u16);

impl Debug for ClientId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{}", self.0))
    }
}

/// An "amount" of an asset. This represents a positive amount of a certain asset, with up to four decimal places.
/// This is used for the amount field of a deposit or withdrawal, it is not used for the total balance of a client which can go negative.
/// The amount is stored as an integer number, preventing rounding errors.
#[derive(Clone, Copy, Default)]
// a u64 is enough to hold almost 30 billion dollars of a relatively weak token like SHIB
pub struct Amount(u64);

impl Debug for Amount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let whole = self.0 / 10_000;
        let cents = self.0 % 10_000;
        f.write_fmt(format_args!("{}.{:04}", whole, cents))
    }
}

/// deserialize from a string with 4 decimal places
impl<'de> Deserialize<'de> for Amount {
    fn deserialize<D>(deserializer: D) -> Result<Amount, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: String = Deserialize::deserialize(deserializer)?;
        let (whole, cents) = if let Some((base, after)) = s.split_once('.') {
            (base, Some(after))
        } else {
            (s.as_str(), None)
        };
        let whole: u64 = whole.parse().map_err(serde::de::Error::custom)?;
        let cents: u64 = match cents {
            Some("") => 0,
            Some(cents) => {
                if cents.len() > 4 || cents.chars().any(|c| !c.is_ascii_digit()) {
                    return Err(serde::de::Error::custom("cents must be at most 4 digits"));
                }

                cents.parse::<u64>().map_err(serde::de::Error::custom)?
                    * 10u64.pow(4 - cents.len() as u32)
            }
            None => 0,
        };
        whole
            .checked_mul(1_00_00)
            .and_then(|whole| whole.checked_add(cents))
            .map(Amount)
            .ok_or_else(|| serde::de::Error::custom("amount too large"))
    }
}

/// serialize as a string with 4 decimal places
impl Serialize for Amount {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let s = format!("{self:?}");
        serializer.serialize_str(&s)
    }
}

/// a balance of funds in an account.
/// A decimal with 4 digits of precision which can go negative.
#[derive(Default, Clone, Copy)]
pub struct Balance(i128);

impl Balance {
    /// try to add an amount to the balance, returning an error if it would overflow.
    /// returns the new balance if successful (it does not modify the original balance).
    #[must_use = "this returns the new balance, it does not modify the original balance"]
    pub fn try_add(self, other: Amount) -> Result<Self> {
        self.0
            .checked_add_unsigned(other.0 as u128)
            .ok_or_else(|| anyhow::anyhow!("overflow updating balance"))
            .map(Self)
    }
    /// try to subtract an amount from the balance, returning an error if it would underflow
    /// returns the new balance if successful (it does not modify the original balance).
    #[must_use = "this returns the new balance, it does not modify the original balance"]
    pub fn try_sub(self, other: Amount) -> Result<Self> {
        self.0
            .checked_sub_unsigned(other.0 as u128)
            .ok_or_else(|| anyhow::anyhow!("underflow updating balance"))
            .map(Self)
    }
}

impl Debug for Balance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let whole = self.0 / 10_000;
        let cents = self.0 % 10_000;
        f.write_fmt(format_args!("{}.{:04}", whole, cents))
    }
}

impl Serialize for Balance {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let s = format!("{self:?}");
        serializer.serialize_str(&s)
    }
}

#[cfg(test)]
mod tests {
    use super::Amount;
    /// ensure the amount in a transaction is always positive, to prevent someone withdrawing negative funds
    #[test]
    fn amount_positive() {
        let entry = "amount\n-1.00\n-0.001\n0.-5";
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .comment(Some(b'#'))
            .flexible(true)
            .trim(csv::Trim::All)
            .from_reader(entry.as_bytes());
        for record in reader.deserialize::<Amount>() {
            assert!(record.is_err());
        }
    }

    /// ensure the amount in a transaction has at most 4 decimal places
    #[test]
    fn amount_precision() {
        let entry = "amount
             1
             1.
             1.0000
             1.00000
             18446744073709551615";
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .comment(Some(b'#'))
            .flexible(true)
            .trim(csv::Trim::All)
            .from_reader(entry.as_bytes());
        let mut records = reader.deserialize::<Amount>();
        assert!(records.next().is_some_and(|x| x.is_ok()));
        assert!(records.next().is_some_and(|x| x.is_ok()));
        assert!(records.next().is_some_and(|x| x.is_ok()));
        assert!(records.next().is_some_and(|x| x.is_err()));
        assert!(records.next().is_some_and(|x| x.is_err()));
        assert!(records.next().is_none());
    }
}
