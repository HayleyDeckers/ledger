use anyhow::Result;
use serde::{Deserialize, Deserializer, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Debug,
    i128,
};

// ✨ we use a newtype pattern to make our code more type-safe and easier to update.
// ✨ By using a new struct instead of `type X = Y;` we block ourselves from accidentally performing arithmetic on the id's.
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

/// A credit of funds to a client's account.
#[derive(Debug)]
pub struct Deposit {
    client_id: ClientId,
    transaction_id: TransactionId,
    amount: Amount,
}

/// A debit of funds from a client's account.
#[derive(Debug)]
pub struct Withdrawal {
    client_id: ClientId,
    transaction_id: TransactionId,
    amount: Amount,
}

/// A dispute of a deposit.
#[derive(Debug)]
pub struct Dispute {
    disputed_transaction: TransactionId,
}

/// A resolution of a dispute.
#[derive(Debug)]
pub struct Resolve {
    disputed_transaction: TransactionId,
}

/// A chargeback of a disputed transaction.
/// This locks the client's account.
#[derive(Debug)]
pub struct Chargeback {
    disputed_transaction: TransactionId,
}

/// An action (transaction) on a client's account.
pub enum AccountAction {
    Deposit(Deposit),
    Withdrawal(Withdrawal),
    Dispute(Dispute),
    Resolve(Resolve),
    Chargeback(Chargeback),
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
                    return Err(serde::de::Error::custom("missing amount"));
                }
            }
            TransactionType::Dispute | TransactionType::Resolve | TransactionType::Chargeback => {
                // amount _must_ be missing for disputes, resolves, and chargebacks
                if amount.is_some() {
                    return Err(serde::de::Error::custom("amount set"));
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
    #[must_use]
    pub fn try_add(self, other: Amount) -> Result<Self> {
        self.0
            .checked_add_unsigned(other.0 as u128)
            .ok_or_else(|| anyhow::anyhow!("overflow updating balance"))
            .map(Self)
    }
    /// try to subtract an amount from the balance, returning an error if it would underflow
    /// returns the new balance if successful (it does not modify the original balance).
    #[must_use]
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

/// A deposit that has been seen by the database.
/// used to lookup transactions for disputes.
#[derive(Debug)]
struct SeenDeposit {
    client_id: ClientId,
    disputed: bool,
    amount: Amount,
}

/// The database of clients and transactions.
/// Keeps track of all seen deposits, transaction ids, and the current state of all clients.
#[derive(Debug, Default)]
pub struct Database {
    // stores all the known clients
    clients: BTreeMap<ClientId, Client>,
    // tracks which transaction ids have been seen
    // we can assume that transaction ids are unique
    // but if they aren't for any reason, the code breaks in weird ways so we include a guard rail to be sure
    //  if this check is implemented in production, we should use a more performant data structure
    // either roaring or range-set-blaze look like good choices here
    // https://github.com/CarlKCarlK/range-set-blaze/blob/main/docs/bench.md
    seen_transactions: BTreeSet<TransactionId>,
    // TransactionId is said to be globally unique, but disputes/resolves/chargebacks actions include a client id in the CSV.
    //  it is unclear what the correct behaviour should be if these disagree with the client id in the deposit/withdrawal.
    // I have opted for ignoring the client id in the dispute/resolve/chargback action, and only using the transaction id.
    //  in the real world, this would be an important detail to clarify with the product owner / docs / upstream team / partner.
    deposit_transactions: BTreeMap<TransactionId, SeenDeposit>,
}

impl Database {
    /// create an empty database.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clients(&self) -> impl Iterator<Item = ClientWithId> {
        self.clients
            .iter()
            .map(|(&id, client)| ClientWithId { id, client })
    }

    /// get a muteable reference to a client by id.
    /// creates the client if it doesn't exist.
    pub fn client_mut(&mut self, id: ClientId) -> &mut Client {
        self.clients.entry(id).or_default()
    }

    fn handle_deposit(&mut self, deposit: Deposit) -> Result<()> {
        let Deposit {
            client_id,
            transaction_id,
            amount,
        } = deposit;
        if !self.seen_transactions.insert(transaction_id) {
            return Err(anyhow::anyhow!("transaction already processed"));
        }
        self.client_mut(client_id).deposit(amount)?;
        self.deposit_transactions.insert(
            transaction_id,
            SeenDeposit {
                disputed: false,
                client_id,
                amount,
            },
        );
        Ok(())
    }

    fn handle_withdrawal(&mut self, withdrawal: Withdrawal) -> Result<()> {
        let Withdrawal {
            client_id,
            transaction_id,
            amount,
        } = withdrawal;
        if !self.seen_transactions.insert(transaction_id) {
            return Err(anyhow::anyhow!("transaction already processed"));
        }
        self.client_mut(client_id).withdraw(amount)?;
        Ok(())
    }

    fn handle_dispute(&mut self, dispute: Dispute) -> Result<()> {
        let Dispute {
            disputed_transaction,
        } = dispute;
        let deposit = self
            .deposit_transactions
            .get_mut(&disputed_transaction)
            .ok_or_else(|| anyhow::anyhow!("deposit not found"))?;
        if deposit.disputed {
            // already disputed, nothing to do
            return Ok(());
        }
        let amount = deposit.amount;
        // we can't use the client function here because of the borrow checker.
        // since Self::client(&mut self) borrows _all_ of self muteable it conflicts with
        // the borrow of deposit_transactions.
        // using this one line works because it only borrows self.client, which doesn't conflict with the borrow of deposit_transactions.
        self.clients
            .entry(deposit.client_id)
            .or_default()
            .hold(amount)?;
        deposit.disputed = true;
        Ok(())
    }

    fn handle_resolve(&mut self, resolve: Resolve) -> Result<()> {
        let Resolve {
            disputed_transaction,
        } = resolve;
        let deposit = self
            .deposit_transactions
            .get_mut(&disputed_transaction)
            .ok_or_else(|| anyhow::anyhow!("deposit not found"))?;
        if !deposit.disputed {
            return Err(anyhow::anyhow!("deposit not disputed"));
        }
        self.clients
            .entry(deposit.client_id)
            .or_default()
            .resolve(deposit.amount)?;
        // a resolved transaction can be disputed again, so we only change the flag
        // and don't remove it from the list of deposits
        deposit.disputed = false;
        Ok(())
    }

    fn handle_chargeback(&mut self, chargeback: Chargeback) -> Result<()> {
        let Chargeback {
            disputed_transaction,
        } = chargeback;
        let deposit = self
            .deposit_transactions
            .get_mut(&disputed_transaction)
            .ok_or_else(|| anyhow::anyhow!("deposit not found"))?;
        if !deposit.disputed {
            return Err(anyhow::anyhow!("deposit not disputed"));
        }
        self.clients
            .entry(deposit.client_id)
            .or_default()
            .chargeback(deposit.amount)?;
        // when a transaction has been charged back, we remove it from the list of deposits
        // to prevent it from being disputed again.
        self.deposit_transactions.remove(&disputed_transaction);
        Ok(())
    }

    /// perform an action on the database.
    ///
    /// for deposits and withdrawals, this will check that the transaction id is unique, or return an error then try to update the client's balance.
    /// Returning an error if it fails to update the balance.
    ///
    /// for disputes, resolves, and chargebacks, this will look up the transaction in the list of deposits and if it exists will try and perform the action returning an error if it fails.
    /// updates to the client's balance are atomic. They will either fully succeed or fully fail.
    pub fn perform_action(&mut self, action: AccountAction) -> Result<()> {
        match action {
            AccountAction::Deposit(deposit) => self.handle_deposit(deposit),
            AccountAction::Withdrawal(withdrawal) => self.handle_withdrawal(withdrawal),
            AccountAction::Dispute(dispute) => self.handle_dispute(dispute),
            AccountAction::Resolve(resolve) => self.handle_resolve(resolve),
            AccountAction::Chargeback(chargeback) => self.handle_chargeback(chargeback),
        }
    }
}

/// A client's account.
///
/// keeps track of the available funds, held funds, and if the account is locked.
#[derive(Debug, Default)]
pub struct Client {
    /// The total funds available for withdrawal etc. This can go negative due to disputes.
    available: Balance,
    /// The total funds that are held for dispute. This should be equal to total - available amounts
    /// and always be positive
    held: Balance,
    locked: bool,
}

impl Client {
    /// Returns the total funds in the account. This is the sum of the available and held funds.
    pub fn total(&self) -> Balance {
        // we don't return an error on overflow here because it should be impossible to even hit this case.
        // if we do manage to overflow here, something has gone _very_ wrong and panicking is the correct response.
        Balance(
            self.available
                .0
                .checked_add(self.held.0)
                .expect("i128 overflow occured when adding held balance to the available balance"),
        )
    }

    /// Returns the held funds in the account. That is, the funds that are currently held for dispute.
    pub fn held(&self) -> Balance {
        self.held
    }

    /// Returns the available funds in the account. That is, the funds that are available for withdrawal.
    pub fn available(&self) -> Balance {
        self.available
    }

    /// Deposit funds into the account.
    ///
    /// this will fail if an overflow occurs.
    fn deposit(&mut self, amount: Amount) -> Result<()> {
        self.available = self.available.try_add(amount)?;
        Ok(())
    }

    /// Withdraw funds from the account.
    ///
    /// this will fail if the account is locked, there are insufficient funds, or an underflow occurs.
    fn withdraw(&mut self, amount: Amount) -> Result<()> {
        if self.locked {
            return Err(anyhow::anyhow!("account is locked"));
        }
        if self.available.0 < amount.0 as i128 {
            return Err(anyhow::anyhow!("insufficient funds"));
        }
        // this line should never fail because we have already checked that available >= amount
        self.available = self.available.try_sub(amount)?;
        Ok(())
    }

    /// Hold funds in the account for dispute.
    /// This will move funds from the available balance to the held balance.
    ///
    /// This function can fail if an overflow or underflow occurs.
    fn hold(&mut self, amount: Amount) -> Result<()> {
        let new_held = self.held.try_add(amount);
        let new_available = self.available.try_sub(amount);
        match (new_held, new_available) {
            (Ok(new_held), Ok(new_available)) => {
                self.held = new_held;
                self.available = new_available;
                Ok(())
            }
            (Err(e), _) | (_, Err(e)) => Err(e),
        }
    }

    /// Resolve a dispute. Making held funds available again.
    ///
    /// This function can fail if an under- or overflow  occurs, or if there are insufficient held funds (If this occurs, there is a bug in the code).
    pub fn resolve(&mut self, amount: Amount) -> Result<()> {
        if self.held.0 < amount.0 as i128 {
            return Err(anyhow::anyhow!(
                "insufficient held funds. Likely a bug in the transaction processing"
            ));
        }
        let new_held = self.held.try_sub(amount);
        let new_available = self.available.try_add(amount);
        match (new_held, new_available) {
            (Ok(new_held), Ok(new_available)) => {
                self.held = new_held;
                self.available = new_available;
                Ok(())
            }
            (Err(e), _) | (_, Err(e)) => Err(e),
        }
    }

    /// Chargeback a dispute. Locking the account.
    ///
    /// This function can fail if and underflow occurs, or there are insufficient held funds (If this occurs, there is a bug in the code).
    /// if this function fails, the account will still be locked.
    pub fn chargeback(&mut self, amount: Amount) -> Result<()> {
        self.locked = true;
        if self.held.0 < amount.0 as i128 {
            return Err(anyhow::anyhow!(
                "insufficient held funds. Likely a bug in the transaction processing"
            ));
        }
        // this line should never fail because we have already checked that held >= amount
        self.held = self.held.try_sub(amount)?;
        Ok(())
    }
}

/// A client with an ID.
///
/// used for serializing the client with the ID.
pub struct ClientWithId<'a> {
    id: ClientId,
    client: &'a Client,
}

impl<'a> Serialize for ClientWithId<'a> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("Client", 5)?;
        state.serialize_field("client", &self.id.0)?;
        state.serialize_field("available", &self.client.available())?;
        state.serialize_field("held", &self.client.held())?;
        state.serialize_field("total", &(self.client.total()))?;
        state.serialize_field("locked", &self.client.locked)?;
        state.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
