#![cfg_attr(not(feature = "std"), no_std)]

use frame_support::{decl_error, decl_event, decl_module, decl_storage, ensure, Parameter};
use rstd::{
	collections::btree_map::BTreeMap,
	convert::{TryFrom, TryInto},
};
use sp_runtime::{
	traits::{CheckedAdd, CheckedSub, MaybeSerializeDeserialize, Member, SimpleArithmetic, StaticLookup},
	DispatchResult,
};
// FIXME: `pallet/frame-` prefix should be used for all pallet modules, but currently `frame_system`
// would cause compiling error in `decl_module!` and `construct_runtime!`
// #3295 https://github.com/paritytech/substrate/issues/3295
use frame_system::{self as system, ensure_signed};

use orml_traits::{
	arithmetic::{self, Signed},
	MultiCurrency, MultiCurrencyExtended,
};

mod mock;
mod tests;

pub trait Trait: frame_system::Trait {
	type Event: From<Event<Self>> + Into<<Self as frame_system::Trait>::Event>;
	type Balance: Parameter + Member + SimpleArithmetic + Default + Copy + MaybeSerializeDeserialize;
	type Amount: Signed
		+ TryInto<Self::Balance>
		+ TryFrom<Self::Balance>
		+ Parameter
		+ Member
		+ arithmetic::SimpleArithmetic
		+ Default
		+ Copy
		+ MaybeSerializeDeserialize;
	type CurrencyId: Parameter + Member + Copy + MaybeSerializeDeserialize + Ord;
}

decl_storage! {
	trait Store for Module<T: Trait> as Tokens {
		/// The total issuance of a token type.
		pub TotalIssuance get(fn total_issuance) build(|config: &GenesisConfig<T>| {
			config
				.endowed_accounts
				.iter()
				.map(|(_, currency_id, initial_balance)| (currency_id, initial_balance))
				.fold(BTreeMap::<T::CurrencyId, T::Balance>::new(), |mut acc, (currency_id, initial_balance)| {
					if let Some(issuance) = acc.get_mut(currency_id) {
						*issuance = issuance.checked_add(initial_balance).expect("total issuance cannot overflow when building genesis");
					} else {
						acc.insert(*currency_id, *initial_balance);
					}
					acc
				})
				.into_iter()
				.collect::<Vec<_>>()
		}): map T::CurrencyId => T::Balance;

		/// The balance of a token type under an account.
		pub Balance get(fn balance): double_map T::CurrencyId, T::AccountId => T::Balance;
	}
	add_extra_genesis {
		config(endowed_accounts): Vec<(T::AccountId, T::CurrencyId, T::Balance)>;

		build(|config: &GenesisConfig<T>| {
			config.endowed_accounts.iter().for_each(|(account_id, currency_id, initial_balance)| {
				<Balance<T>>::insert(currency_id, account_id, initial_balance);
			})
		})
	}
}

decl_event!(
	pub enum Event<T> where
		<T as frame_system::Trait>::AccountId,
		<T as Trait>::CurrencyId,
		<T as Trait>::Balance
	{
		/// Token transfer success (currency_id, from, to, amount)
		Transferred(CurrencyId, AccountId, AccountId, Balance),
	}
);

decl_module! {
	pub struct Module<T: Trait> for enum Call where origin: T::Origin {
		type Error = Error<T>;

		fn deposit_event() = default;

		/// Transfer some balance to another account.
		pub fn transfer(
			origin,
			dest: <T::Lookup as StaticLookup>::Source,
			currency_id: T::CurrencyId,
			#[compact] amount: T::Balance,
		) {
			let from = ensure_signed(origin)?;
			let to = T::Lookup::lookup(dest)?;
			<Self as MultiCurrency<_>>::transfer(currency_id, &from, &to, amount)?;

			Self::deposit_event(RawEvent::Transferred(currency_id, from, to, amount));
		}
	}
}

decl_error! {
	/// Error for token module.
	pub enum Error for Module<T: Trait> {
		BalanceTooLow,
		TotalIssuanceOverflow,
		AmountIntoBalanceFailed,
	}
}

impl<T: Trait> Module<T> {}

impl<T: Trait> MultiCurrency<T::AccountId> for Module<T> {
	type CurrencyId = T::CurrencyId;
	type Balance = T::Balance;

	fn total_issuance(currency_id: Self::CurrencyId) -> Self::Balance {
		<TotalIssuance<T>>::get(currency_id)
	}

	fn balance(currency_id: Self::CurrencyId, who: &T::AccountId) -> Self::Balance {
		<Balance<T>>::get(currency_id, who)
	}

	fn ensure_can_withdraw(currency_id: Self::CurrencyId, who: &T::AccountId, amount: Self::Balance) -> DispatchResult {
		if Self::balance(currency_id, who).checked_sub(&amount).is_some() {
			Ok(())
		} else {
			Err(Error::<T>::BalanceTooLow.into())
		}
	}

	fn transfer(
		currency_id: Self::CurrencyId,
		from: &T::AccountId,
		to: &T::AccountId,
		amount: Self::Balance,
	) -> DispatchResult {
		ensure!(Self::balance(currency_id, from) >= amount, Error::<T>::BalanceTooLow);

		if from != to {
			<Balance<T>>::mutate(currency_id, from, |balance| *balance -= amount);
			<Balance<T>>::mutate(currency_id, to, |balance| *balance += amount);
		}

		Ok(())
	}

	fn deposit(currency_id: Self::CurrencyId, who: &T::AccountId, amount: Self::Balance) -> DispatchResult {
		ensure!(
			Self::total_issuance(currency_id).checked_add(&amount).is_some(),
			Error::<T>::TotalIssuanceOverflow,
		);

		<TotalIssuance<T>>::mutate(currency_id, |v| *v += amount);
		<Balance<T>>::mutate(currency_id, who, |v| *v += amount);

		Ok(())
	}

	fn withdraw(currency_id: Self::CurrencyId, who: &T::AccountId, amount: Self::Balance) -> DispatchResult {
		ensure!(
			Self::balance(currency_id, who).checked_sub(&amount).is_some(),
			Error::<T>::BalanceTooLow,
		);

		<TotalIssuance<T>>::mutate(currency_id, |v| *v -= amount);
		<Balance<T>>::mutate(currency_id, who, |v| *v -= amount);

		Ok(())
	}

	fn slash(currency_id: Self::CurrencyId, who: &T::AccountId, amount: Self::Balance) -> Self::Balance {
		let slashed_amount = Self::balance(currency_id, who).min(amount);
		<TotalIssuance<T>>::mutate(currency_id, |v| *v -= slashed_amount);
		<Balance<T>>::mutate(currency_id, who, |v| *v -= slashed_amount);
		amount - slashed_amount
	}
}

impl<T: Trait> MultiCurrencyExtended<T::AccountId> for Module<T> {
	type Amount = T::Amount;

	fn update_balance(currency_id: Self::CurrencyId, who: &T::AccountId, by_amount: Self::Amount) -> DispatchResult {
		let by_balance =
			TryInto::<Self::Balance>::try_into(by_amount.abs()).map_err(|_| Error::<T>::AmountIntoBalanceFailed)?;
		if by_amount.is_positive() {
			Self::deposit(currency_id, who, by_balance)
		} else {
			Self::withdraw(currency_id, who, by_balance)
		}
	}
}
