use crate::{Amount, Balance, Error, Result};

/// A client's account.
///
/// keeps track of the available funds, held funds, and if the account is locked.
#[derive(Debug, Default)]
pub struct Client {
    /// The total funds available for withdrawal etc. This can go negative due to disputes.
    pub(crate) available: Balance,
    /// The total funds that are held for dispute. This should be equal to total - available amounts
    /// and always be positive
    pub(crate) held: Balance,
    pub(crate) locked: bool,
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

    /// whether the account is locked.
    ///
    /// a locked account can no longer make any withdrawals.
    pub fn is_locked(&self) -> bool {
        self.locked
    }

    /// Deposit funds into the account.
    ///
    /// this will fail if an overflow occurs.
    pub(crate) fn deposit(&mut self, amount: Amount) -> Result<()> {
        self.available = self.available.try_add(amount)?;
        Ok(())
    }

    /// Withdraw funds from the account.
    ///
    /// this will fail if the account is locked, there are insufficient funds, or an underflow occurs.
    pub(crate) fn withdraw(&mut self, amount: Amount) -> Result<()> {
        if self.is_locked() {
            return Err(Error::AccountLocked);
        }
        if self.available.0 < amount.0 as i128 {
            return Err(Error::InsufficientFunds);
        }
        // this line should never fail because we have already checked that available >= amount
        self.available = self.available.try_sub(amount)?;
        Ok(())
    }

    /// Hold funds in the account for dispute.
    /// This will move funds from the available balance to the held balance.
    ///
    /// This function can fail if an overflow or underflow occurs.
    pub(crate) fn hold(&mut self, amount: Amount) -> Result<()> {
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
    pub(crate) fn resolve(&mut self, amount: Amount) -> Result<()> {
        if self.held.0 < amount.0 as i128 {
            return Err(Error::InsufficientHeldFunds);
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
    pub(crate) fn chargeback(&mut self, amount: Amount) -> Result<()> {
        self.locked = true;
        if self.held.0 < amount.0 as i128 {
            return Err(Error::InsufficientHeldFunds);
        }
        // this line should never fail because we have already checked that held >= amount
        self.held = self.held.try_sub(amount)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Amount, Balance, Client};

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
}
