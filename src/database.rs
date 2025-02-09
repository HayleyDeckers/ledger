use crate::{
    actions::{AccountAction, Chargeback, Deposit, Dispute, Resolve, Withdrawal},
    client::Client,
    Amount, ClientId, Result, TransactionId,
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Deref,
};

/// A deposit that has been seen by the database.
/// used to lookup transactions for disputes.
#[derive(Debug)]
pub(crate) struct SeenDeposit {
    client_id: ClientId,
    disputed: bool,
    amount: Amount,
}

/// A client with an ID.
///
/// used for serializing the client with the ID.
pub struct ClientWithId<'a> {
    id: ClientId,
    client: &'a Client,
}

impl ClientWithId<'_> {
    pub fn id(&self) -> ClientId {
        self.id
    }
}

impl Deref for ClientWithId<'_> {
    type Target = Client;

    fn deref(&self) -> &Self::Target {
        self.client
    }
}

impl Serialize for ClientWithId<'_> {
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
        state.serialize_field("locked", &self.client.is_locked())?;
        state.end()
    }
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

    /// returns an iterator over all clients in the database and their associated id.
    /// this is used for serializing the clients.
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

#[cfg(test)]
mod tests {
    use super::*;

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
