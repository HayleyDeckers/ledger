use crate::{Amount, Balance, Result};

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
    pub(crate) fn chargeback(&mut self, amount: Amount) -> Result<()> {
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
