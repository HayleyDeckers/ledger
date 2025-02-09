use crate::{Amount, ClientId, TransactionId};
use serde::{Deserialize, Deserializer};
use std::fmt::Debug;

/// An action (transaction) on a client's account.
pub enum AccountAction {
    Deposit(Deposit),
    Withdrawal(Withdrawal),
    Dispute(Dispute),
    Resolve(Resolve),
    Chargeback(Chargeback),
}

/// A credit of funds to a client's account.
#[derive(Debug)]
pub struct Deposit {
    pub(crate) client_id: ClientId,
    pub(crate) transaction_id: TransactionId,
    pub(crate) amount: Amount,
}

/// A debit of funds from a client's account.
#[derive(Debug)]
pub struct Withdrawal {
    pub(crate) client_id: ClientId,
    pub(crate) transaction_id: TransactionId,
    pub(crate) amount: Amount,
}

/// A dispute of a deposit.
#[derive(Debug)]
pub struct Dispute {
    pub(crate) disputed_transaction: TransactionId,
}

/// A resolution of a dispute.
#[derive(Debug)]
pub struct Resolve {
    pub(crate) disputed_transaction: TransactionId,
}

/// A chargeback of a disputed transaction.
/// This locks the client's account.
#[derive(Debug)]
pub struct Chargeback {
    pub(crate) disputed_transaction: TransactionId,
}

impl Debug for AccountAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AccountAction::Deposit(deposit) => f.write_fmt(format_args!("{:?}", deposit)),
            AccountAction::Withdrawal(withdrawal) => f.write_fmt(format_args!("{:?}", withdrawal)),
            AccountAction::Dispute(dispute) => f.write_fmt(format_args!("{:?}", dispute)),
            AccountAction::Resolve(resolve) => f.write_fmt(format_args!("{:?}", resolve)),
            AccountAction::Chargeback(chargeback) => f.write_fmt(format_args!("{:?}", chargeback)),
        }
    }
}

impl<'de> Deserialize<'de> for AccountAction {
    fn deserialize<D>(deserializer: D) -> Result<AccountAction, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "lowercase")]
        enum TransactionType {
            Deposit,
            Withdrawal,
            Dispute,
            Resolve,
            Chargeback,
        }

        #[derive(Deserialize)]
        struct TransactionRecord {
            //https://github.com/BurntSushi/rust-csv/issues/354 applies here unfortunately
            #[serde(rename = "type")]
            kind: TransactionType,
            client: u16,
            tx: u32,
            amount: Option<Amount>,
        }
        let TransactionRecord {
            kind,
            client,
            tx,
            amount,
        } = TransactionRecord::deserialize(deserializer)?;

        match kind {
            TransactionType::Deposit | TransactionType::Withdrawal => {
                // amount _is_ allowed to be zero, but not missing, for deposits and withdrawals
                if amount.is_none() {
                    return Err(serde::de::Error::custom(
                        "missing amount for deposit or withdrawal",
                    ));
                }
            }
            TransactionType::Dispute | TransactionType::Resolve | TransactionType::Chargeback => {
                // amount _must_ be missing for disputes, resolves, and chargebacks
                if amount.is_some() {
                    return Err(serde::de::Error::custom(
                        "amount set for dispute, resolve, or chargeback",
                    ));
                }
            }
        };

        Ok(match kind {
            TransactionType::Deposit => AccountAction::Deposit(Deposit {
                client_id: ClientId(client),
                transaction_id: TransactionId(tx),
                amount: amount.unwrap(),
            }),
            TransactionType::Withdrawal => AccountAction::Withdrawal(Withdrawal {
                client_id: ClientId(client),
                transaction_id: TransactionId(tx),
                amount: amount.unwrap(),
            }),
            TransactionType::Dispute => AccountAction::Dispute(Dispute {
                disputed_transaction: TransactionId(tx),
            }),
            TransactionType::Resolve => AccountAction::Resolve(Resolve {
                disputed_transaction: TransactionId(tx),
            }),
            TransactionType::Chargeback => AccountAction::Chargeback(Chargeback {
                disputed_transaction: TransactionId(tx),
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::AccountAction;
    /// ensure the amount field must be present for deposits and withdrawals
    #[test]
    fn amount_present() {
        let entry = "type,client,tx,amount\nwithdrawal,1,1\ndeposit,1,2,\ndeposit,1,3,1\n";
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .comment(Some(b'#'))
            .flexible(true)
            .trim(csv::Trim::All)
            .from_reader(entry.as_bytes());
        let mut records = reader.deserialize::<AccountAction>();
        assert!(records.next().is_some_and(|x| x.is_err()));
        assert!(records.next().is_some_and(|x| x.is_err()));
        assert!(records.next().is_some_and(|x| x.is_ok()));
        assert!(records.next().is_none());
    }

    /// ensure the amount field must be missing for disputes, resolves, and chargebacks
    #[test]
    fn amount_missing() {
        let entry = "type,client,tx,amount\ndispute,1,1,1\nresolve,1,2,1\nchargeback,1,3,1\ndispute,1,4,\nresolve,1,5\nchargeback,1,6,\n";
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .comment(Some(b'#'))
            .flexible(true)
            .trim(csv::Trim::All)
            .from_reader(entry.as_bytes());
        let mut records = reader.deserialize::<AccountAction>();
        assert!(records.next().is_some_and(|x| x.is_err()));
        assert!(records.next().is_some_and(|x| x.is_err()));
        assert!(records.next().is_some_and(|x| x.is_err()));
        assert!(records.next().is_some_and(|x| x.is_ok()));
        assert!(records.next().is_some_and(|x| x.is_ok()));
        assert!(records.next().is_some_and(|x| x.is_ok()));
        assert!(records.next().is_none());
    }
}
