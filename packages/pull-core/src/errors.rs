use pinocchio::program_error::ProgramError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum PullError {
    /// Account expected to be a signer
    NotSigner,
    /// An account this program writes directly must be writable
    NotMutable,
    /// Account expected to be owned by this program
    InvalidAccountOwner,
    /// The account data length is not the expected one
    InvalidAccountLength,
    /// The account's tag is not the expected type
    InvalidTag,
    /// The account is not the PDA for its seeds
    InvalidSeeds,
    /// The policy exists already
    AlreadyInitialized,

    /// The terms bytes are not a canonical encoding
    MalformedTerms,
    /// The terms break a validity rule (`Terms::validate`)
    InvalidTerms,
    /// The terms are for another cluster
    WrongCluster,
    /// The signer is not the terms' authority
    InvalidAuthority,
    /// The signature does not cover the rendered terms
    InvalidSignature,
    /// The intent's nonce was used: it ran already, or was cancelled
    NonceUsed,
    /// A timestamp outside years 1970–9999 cannot be rendered
    Unrenderable,

    /// The terms' window has not opened
    NotYetValid,
    /// The terms' window has closed
    Expired,
    /// The signer is not the terms' spender
    InvalidSpender,
    /// Only the authority or the spender may close a policy before it expires
    NotClosable,
    /// The account is not the one that paid the rent being refunded
    InvalidPayer,

    /// The account pulled from is not the one the terms limit
    InvalidPull,
    /// The pull exceeds the limit
    LimitExceeded,
    /// A token account or mint is not the one the terms name, or not the authority's
    InvalidTarget,
    /// An amount overflowed
    Overflow,
    /// The authority received less than the terms require
    NotReceived,
    /// Only the engine PDA may invoke the event instruction
    InvalidEventAuthority,

    /// The engine is not the delegate of the account pulled from: it was never
    /// enabled, another `Approve` replaced it, or its approval ran out
    NotDelegate,
    /// The pull exceeds what is left of the account's approval
    AllowanceExceeded,
    /// The account pulled from holds less than the pull
    InsufficientFunds,
}

impl PullError {
    /// Every error in code order, so clients can name a code: `ALL[code]`.
    /// A new variant goes here too; the test below checks the order.
    pub const ALL: [PullError; 28] = [
        PullError::NotSigner,
        PullError::NotMutable,
        PullError::InvalidAccountOwner,
        PullError::InvalidAccountLength,
        PullError::InvalidTag,
        PullError::InvalidSeeds,
        PullError::AlreadyInitialized,
        PullError::MalformedTerms,
        PullError::InvalidTerms,
        PullError::WrongCluster,
        PullError::InvalidAuthority,
        PullError::InvalidSignature,
        PullError::NonceUsed,
        PullError::Unrenderable,
        PullError::NotYetValid,
        PullError::Expired,
        PullError::InvalidSpender,
        PullError::NotClosable,
        PullError::InvalidPayer,
        PullError::InvalidPull,
        PullError::LimitExceeded,
        PullError::InvalidTarget,
        PullError::Overflow,
        PullError::NotReceived,
        PullError::InvalidEventAuthority,
        PullError::NotDelegate,
        PullError::AllowanceExceeded,
        PullError::InsufficientFunds,
    ];
}

impl From<PullError> for ProgramError {
    #[inline]
    fn from(e: PullError) -> Self {
        ProgramError::Custom(e as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::PullError;

    #[test]
    fn all_lists_every_error_in_code_order() {
        for (code, error) in PullError::ALL.iter().enumerate() {
            assert_eq!(*error as usize, code);
        }
    }
}
