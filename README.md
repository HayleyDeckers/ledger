## ledger

This crate implements a toy payment engine that processes CSV files containing deposits, withdrawals, disputes, chargebacks, and dispute resolutions.

### Design

This crate strives to implement all documented features and maintain a correct ledger at all times.

The main entry point is the function `Database::perform_action(&mut self, action: AccountAction)`. This function takes a single `AccountAction` (corresponding to a single row of CSV data), and attempts to apply it. It may fail for various reasons (detailed in the code), such as underflow or overflow when updating client balances or attempting to overdraw.

The `Database` maintains a record of all transaction IDs (from withdrawals and deposits) and will raise an error if a duplicate is encountered. It also tracks all deposits and their dispute status. Additionally, it stores a map of `Client` records, where each `Client` holds its available and held balances and indicates whether the account is locked.

Throughout the crate, strong typing is employed to reduce errors. For example, IDs are wrapped in new types to prevent unintended operations (e.g., accidental use of `ops::Add`). The types for deposit and withdrawal amounts (`Amount`) wrap a `u64` ensuring amounts cannot be negative, while client balances use `i128`. By using integers instead of floats we prevent rounding errors and by checking all arithmetic operations performed on balances we prevent over- or underflow. Furthermore, when updating a client’s funds for a hold operation, the library guarantees that either both the available and held funds are updated successfully or neither is changed.

Unit tests are present in `lib.rs` to validate the assumptions made and guarantees provided by the crate.

---

### Additional Assumptions

- The `client` field in dispute, resolve, or chargeback actions is ignored. It is unspecified whether the client ID must match the one associated with the disputed transaction (`tx`). In production, validating this would be essential.
- Disputes can only be applied to deposits. While disputing withdrawals could be a useful feature in case a client is hacked or scammed, it does not seem to be in scope for this assigment.
- A frozen account cannot withdraw funds but can still accept deposits or have its deposits disputed/resolved/chargedback.
- A client's available balance may become negative, but only as a result of a chargeback. (Alternatively, one might block chargebacks if insufficient funds exist. Which behavior is correct depends on the agreement with the counter-party.)
