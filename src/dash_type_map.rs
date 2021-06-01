use std::any::*;
use std::future::Future;
use std::option::Option::None;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

#[derive(Debug)]
pub enum DashTypeMapErrors {
	AlreadyExists,
	DoesNotExist,
	Timeout,
}

impl std::error::Error for DashTypeMapErrors {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		use DashTypeMapErrors::*;
		match self {
			AlreadyExists => None,
			DoesNotExist => None,
			Timeout => None,
		}
	}
}

impl std::fmt::Display for DashTypeMapErrors {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		use DashTypeMapErrors::*;
		match self {
			AlreadyExists => f.write_str("already exists"),
			DoesNotExist => f.write_str("does not exist"),
			Timeout => f.write_str("timeout"),
		}
	}
}

#[derive(Debug, Default)]
pub struct DashTypeMap(
	dashmap::DashMap<TypeId, Box<dyn Any + Send + Sync>>,
	crossbeam::queue::SegQueue<Waker>,
);

impl DashTypeMap {
	pub fn new() -> Self {
		Default::default()
	}

	pub fn as_dashmap(
		&self,
	) -> &dashmap::DashMap<std::any::TypeId, Box<dyn std::any::Any + Send + Sync>> {
		&self.0
	}

	pub fn add_change_waker(&self, waker: Waker) {
		self.1.push(waker);
	}

	fn process_change_wakers(&self) {
		while let Some(waker) = self.1.pop() {
			waker.wake();
		}
	}

	pub fn contains<K: 'static>(&self) -> bool {
		self.contains_key(&TypeId::of::<K>())
	}

	pub fn contains_key(&self, key: &TypeId) -> bool {
		self.0.contains_key(key)
	}

	pub fn insert<V: 'static + Send + Sync>(
		&self,
		value: impl Into<Box<V>>,
	) -> Result<(), DashTypeMapErrors> {
		let key = TypeId::of::<V>();
		if self.contains_key(&key) {
			return Err(DashTypeMapErrors::AlreadyExists);
		}
		self.0.insert(key, value.into());
		self.process_change_wakers();
		Ok(())
	}

	pub fn remove<V: 'static + Send + Sync>(&self) -> Result<Box<V>, DashTypeMapErrors> {
		let (_key, value) = self
			.0
			.remove(&TypeId::of::<V>())
			.ok_or(DashTypeMapErrors::DoesNotExist)?;
		self.process_change_wakers();
		let value = value
			.downcast::<V>()
			.expect("internal data state failure, any type does not match actual type");
		Ok(value)
	}

	pub fn with<V: 'static + Send + Sync, R, F: FnOnce(&V) -> R>(
		&self,
		fun: F,
	) -> Result<R, DashTypeMapErrors> {
		let value = self
			.0
			.get(&TypeId::of::<V>())
			.ok_or(DashTypeMapErrors::DoesNotExist)?;
		let value = &**value;
		let value = value
			.downcast_ref::<V>()
			.expect("internal data state failure, any type does not match actual type");
		let ret = fun(value);
		Ok(ret)
	}

	pub fn with_mut<V: 'static + Send + Sync, R, F: FnOnce(&mut V) -> R>(
		&self,
		fun: F,
	) -> Result<R, DashTypeMapErrors> {
		let mut value = self
			.0
			.get_mut(&TypeId::of::<V>())
			.ok_or(DashTypeMapErrors::DoesNotExist)?;
		let value = &mut **value;
		let value = value
			.downcast_mut::<V>()
			.expect("internal data state failure, any type does not match actual type");
		let ret = fun(value);
		Ok(ret)
	}

	pub fn clone_if_arc<V: 'static + Send + Sync>(&self) -> Result<Arc<V>, DashTypeMapErrors> {
		self.with::<Arc<V>, Arc<V>, _>(Clone::clone)
	}

	pub fn wait_for_existence_of(
		&self,
		key: TypeId,
		timeout: Duration,
	) -> DashTypeMapWaiterExistence {
		DashTypeMapWaiterExistence(Box::pin(tokio::time::sleep(timeout)), self, key, true)
	}

	pub fn wait_for_removal_of(
		&self,
		key: TypeId,
		timeout: Duration,
	) -> DashTypeMapWaiterExistence {
		DashTypeMapWaiterExistence(Box::pin(tokio::time::sleep(timeout)), self, key, false)
	}

	pub async fn wait_clone_if_arc<V: 'static + Send + Sync>(
		&self,
		timeout: Duration,
	) -> Result<Arc<V>, DashTypeMapErrors> {
		let mut arc = self.clone_if_arc::<V>();
		while let Err(DashTypeMapErrors::DoesNotExist) = arc {
			if !self
				.wait_for_existence_of(TypeId::of::<Arc<V>>(), timeout)
				.await
			{
				return Err(DashTypeMapErrors::Timeout);
			}
			arc = self.clone_if_arc::<V>()
		}
		arc
	}
}

pub struct DashTypeMapWaiterExistence<'s>(
	Pin<Box<tokio::time::Sleep>>,
	&'s DashTypeMap,
	TypeId,
	bool,
);

impl<'s> Future for DashTypeMapWaiterExistence<'s> {
	type Output = bool;

	fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		if self.1.contains_key(&self.2) == self.3 {
			Poll::Ready(true)
		} else {
			match self.0.as_mut().poll(cx) {
				Poll::Ready(()) => Poll::Ready(false),
				Poll::Pending => {
					self.1.add_change_waker(cx.waker().clone());
					Poll::Pending
				}
			}
		}
	}
}
