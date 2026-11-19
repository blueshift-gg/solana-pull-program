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
    /// The profile exists already
    AlreadyInitialized,

    /// The terms bytes are not a canonical encoding
    MalformedTerms,
    /// The terms break a validity rule (`Terms::validate`)
    InvalidTerms,

    /// The terms' window has not opened
    NotYetValid,
    /// The terms' window has closed
    Expired,
    /// The signer is not the terms' spender
    InvalidSpender,
    /// A mandate that can still pull: only its authority closes it
    NotClosable,
    /// The account is not the one that paid the rent being refunded
    InvalidPayer,

    /// The account pulled from is not the one the terms limit
    InvalidPull,
    /// The pull exceeds what one pull may take, or what is left of the limit
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

    /// The account is not the authority's profile, or the authority has none
    InvalidProfile,
    /// The authority invalidated this mandate
    Stale,
    /// The tokens would not go to the account the terms name
    InvalidDestination,
    /// A pull must take something
    ZeroAmount,
}

impl From<PullError> for ProgramError {
    #[inline]
    fn from(e: PullError) -> Self {
        ProgramError::Custom(e as u32)
    }
}
