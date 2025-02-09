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
    use super::{actions::*, client::Client, database::Database, *};
    /// ensure the amount in a transaction is always positive, to prevent someone withdrawing negative funds
    #[test]
    fn amount_positive() {
        let entry = "type,client,tx,amount\nwithdrawal,1,1,-1.00\ndeposit,1,2,-1.00";
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .comment(Some(b'#'))
            .flexible(true)
            .trim(csv::Trim::All)
            .from_reader(entry.as_bytes());
        for record in reader.deserialize::<AccountAction>() {
            assert!(record.is_err());
        }
    }

    /// ensure the amount in a transaction has at most 4 decimal places
    #[test]
    fn amount_precision() {
        let entry = "type,client,tx,amount
             deposit,1,1,1
             deposit,1,1,1.
             deposit,1,1,1.0000
             deposit,1,1,1.00000
             deposit,1,1,18446744073709551615";
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .comment(Some(b'#'))
            .flexible(true)
            .trim(csv::Trim::All)
            .from_reader(entry.as_bytes());
        let mut records = reader.deserialize::<AccountAction>();
        assert!(records.next().is_some_and(|x| x.is_ok()));
        assert!(records.next().is_some_and(|x| x.is_ok()));
        assert!(records.next().is_some_and(|x| x.is_ok()));
        assert!(records.next().is_some_and(|x| x.is_err()));
        assert!(records.next().is_some_and(|x| x.is_err()));
        assert!(records.next().is_none());
    }

    /// ensure a user can't withdraw into the negative
    #[test]
    fn withdrawal_negative() {
        let mut client = Client::default();
        client.available = Balance(5);
        assert!(client.withdraw(Amount(10)).is_err());
        assert!(client.available.0 == 5);

        client.available = Balance(-5);
        assert!(client.withdraw(Amount(5)).is_err());
    }

    /// ensure no over- or under-flow can occur when applying mutations to the balance
    #[test]
    fn balance_overflow() {
        let mut client = Client::default();
        // overflowing a deposit
        client.available = Balance(i128::MAX);
        assert!(client.deposit(Amount(1)).is_err());
        assert!(client.available.0 == i128::MAX);

        // underflowing a withdrawal
        // actually stopped before the underflow because of insufficient funds
        client.available = Balance(i128::MIN);
        assert!(client.withdraw(Amount(1)).is_err());
        assert!(client.available.0 == i128::MIN);

        // underflowing a hold
        // (can occur if we dispute more than the client has available)
        client.available = Balance(i128::MIN);
        assert!(client.hold(Amount(1)).is_err());
        assert!(client.available.0 == i128::MIN);
        assert!(client.held.0 == 0);

        // overflowing a resolve
        client.available = Balance(i128::MAX);
        client.held = Balance(1);
        assert!(client.resolve(Amount(1)).is_err());

        // underflowing a chargeback
        // actually stopped before the underflow because of insufficient held funds
        client.held = Balance(i128::MIN);
        assert!(client.chargeback(Amount(1)).is_err());
    }

    /// ensure that the total balance is always equal to the sum of the available and held balances
    #[test]
    fn total_balance() {
        let mut client = Client::default();
        client.available = Balance(-5);
        client.held = Balance(2);
        assert!(client.total().0 == -3);
    }

    /// ensure that we can put a hold on a client even if they have negative funds
    #[test]
    fn hold_negative() {
        let mut client = Client::default();
        client.available = Balance(-5);
        client.held = Balance(0);
        assert!(client.hold(Amount(5)).is_ok());
        assert!(client.available.0 == -10);
        assert!(client.held.0 == 5);
    }

    /// ensure disputes can only target deposits
    #[test]
    fn dispute_target() {
        let mut db = Database::new();
        let deposit = Deposit {
            client_id: ClientId(1),
            transaction_id: TransactionId(1),
            amount: Amount(1),
        };
        let withdrawal = Withdrawal {
            client_id: ClientId(1),
            transaction_id: TransactionId(2),
            amount: Amount(1),
        };
        let dispute = Dispute {
            disputed_transaction: TransactionId(2),
        };
        assert!(db.perform_action(AccountAction::Deposit(deposit)).is_ok());
        assert!(db
            .perform_action(AccountAction::Withdrawal(withdrawal))
            .is_ok());
        assert!(db.perform_action(AccountAction::Dispute(dispute)).is_err());
    }

    /// ensure locked accounts can't be withdrawn from
    #[test]
    fn locked_withdraw() {
        let mut client = Client::default();
        client.available = Balance(1);
        client.locked = true;
        assert!(client.withdraw(Amount(1)).is_err());
        assert!(client.available.0 == 1);
        client.locked = false;
        assert!(client.withdraw(Amount(1)).is_ok());
        assert!(client.available.0 == 0);
    }

    /// ensure that a dispute can't be resolved if the funds are insufficient
    /// (this should never happen in production, it would be a bug in the transaction processing)
    #[test]
    fn resolve_insufficient() {
        let mut client = Client::default();
        client.held = Balance(1);
        assert!(client.resolve(Amount(2)).is_err());
        assert!(client.held.0 == 1);
    }

    /// ensure that a chargeback can't be performed if the funds are insufficient
    /// (this should never happen in production, it would be a bug in the transaction processing)
    #[test]
    fn chargeback_insufficient() {
        let mut client = Client::default();
        client.held = Balance(1);
        assert!(client.chargeback(Amount(2)).is_err());
        assert!(client.held.0 == 1);
    }

    /// ensure that a chargeback locks the account
    #[test]
    fn chargeback_lock() {
        let mut client = Client::default();
        client.held = Balance(1);
        assert!(client.chargeback(Amount(1)).is_ok());
        assert!(client.locked);
    }

    /// ensure that a resolved dispute can be disputed again
    #[test]
    fn redispute() {
        let mut client = Client::default();
        client.available = Balance(1);
        assert!(client.hold(Amount(1)).is_ok());
        assert!(client.resolve(Amount(1)).is_ok());
        assert!(client.held.0 == 0);
        assert!(client.available.0 == 1);
        assert!(client.hold(Amount(1)).is_ok());
    }

    /// ensure that transactions can't be processed twice
    #[test]
    fn duplicate_transaction() {
        let mut db = Database::new();

        assert!(db
            .perform_action(AccountAction::Deposit(Deposit {
                client_id: ClientId(1),
                transaction_id: TransactionId(1),
                amount: Amount(1),
            }))
            .is_ok());
        assert!(db
            .perform_action(AccountAction::Deposit(Deposit {
                client_id: ClientId(1),
                transaction_id: TransactionId(1),
                amount: Amount(1),
            }))
            .is_err());
        assert!(db
            .perform_action(AccountAction::Withdrawal(Withdrawal {
                client_id: ClientId(1),
                transaction_id: TransactionId(1),
                amount: Amount(1),
            }))
            .is_err());
        assert!(db
            .perform_action(AccountAction::Withdrawal(Withdrawal {
                client_id: ClientId(1),
                transaction_id: TransactionId(2),
                amount: Amount(1),
            }))
            .is_ok());
    }

    ///ensure that a deposit can not be charged back multiple times
    #[test]
    fn duplicate_chargeback() {
        let mut db = Database::new();
        assert!(db
            .perform_action(AccountAction::Deposit(Deposit {
                client_id: ClientId(1),
                transaction_id: TransactionId(1),
                amount: Amount(1),
            }))
            .is_ok());
        assert!(db
            .perform_action(AccountAction::Dispute(Dispute {
                disputed_transaction: TransactionId(1),
            }))
            .is_ok());
        assert!(db
            .perform_action(AccountAction::Chargeback(Chargeback {
                disputed_transaction: TransactionId(1),
            }))
            .is_ok());
        assert!(db
            .perform_action(AccountAction::Chargeback(Chargeback {
                disputed_transaction: TransactionId(1),
            }))
            .is_err());
    }

    /// ensure that a chargeback requires a dispute
    #[test]
    fn chargeback_no_dispute() {
        let mut db = Database::new();
        assert!(db
            .perform_action(AccountAction::Deposit(Deposit {
                client_id: ClientId(1),
                transaction_id: TransactionId(1),
                amount: Amount(1),
            }))
            .is_ok());
        assert!(db
            .perform_action(AccountAction::Chargeback(Chargeback {
                disputed_transaction: TransactionId(1),
            }))
            .is_err());
    }

    /// ensure that we can't "resolve" a deposit if it hasn't been disputed
    #[test]
    fn resolve_no_dispute() {
        let mut db = Database::new();
        assert!(db
            .perform_action(AccountAction::Deposit(Deposit {
                client_id: ClientId(1),
                transaction_id: TransactionId(1),
                amount: Amount(1),
            }))
            .is_ok());
        assert!(db
            .perform_action(AccountAction::Resolve(Resolve {
                disputed_transaction: TransactionId(1),
            }))
            .is_err());
    }
}
