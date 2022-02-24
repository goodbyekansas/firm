//! Module for working with `Channels`.
//!
//! A `ChannelSet` is a collection of named `Channel`s and a
//! `Channel` represents a single stream of typed data that can be used as
//! an input or output for a function.

use std::{
    collections::HashMap,
    fmt::{Debug, Display},
    future::Future,
    ops::{Deref, DerefMut},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll, Waker},
};

use firm_types::functions::{
    channel::Value as ProtoValue, Channel as ProtoChannel, ChannelSpec as ProtoChannelSpec,
    ChannelType as ProtoChannelType, Stream as ProtoChannelSet,
};
use futures::{FutureExt, StreamExt, TryFutureExt};
use thiserror::Error;
use tokio::{
    io::AsyncSeek,
    sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard},
};

#[derive(Error, Debug)]
pub enum ChannelSetError {
    #[error("Failed to append output \"{0}\": {1}")]
    FailedToAppendOutput(String, ChannelError),

    #[error("Channel \"{0}\" does not exist.")]
    NonExistingChannel(String),
}

#[derive(Debug, Error)]
pub enum ChannelSetValidationError {
    #[error(
        "Channel \"{channel_name}\" has unexpected type. Expected \"{expected}\", got \"{got}\""
    )]
    MismatchedChannelType {
        channel_name: String,
        expected: String,
        got: String,
    },

    #[error("Channel \"{0}\" was not expected by spec")]
    UnexpectedChannel(String),

    #[error("Failed to find required channel {0}")]
    RequiredChannelMissing(String),
}

/// A collection of channels with a name, can be used as output for a function
#[derive(Debug, PartialEq)]
pub struct ChannelSet<T = ChannelWriter>(HashMap<String, Channel>, T);

/// Marker trait to generalize over [`ChannelSet`]s with read capabilities.
pub trait ChannelRead {
    fn new() -> Self;
}

/// Marker type for [`ChannelSet`]s with read capabilities.
#[derive(Debug, PartialEq)]
pub struct ChannelReader();

/// Marker type for [`ChannelSet`]s with write and read capabilities.
#[derive(Debug, PartialEq)]
pub struct ChannelWriter();

impl ChannelRead for ChannelWriter {
    fn new() -> Self {
        Self()
    }
}

impl ChannelRead for ChannelReader {
    fn new() -> Self {
        Self()
    }
}

impl<T> ChannelSet<T>
where
    T: ChannelRead,
{
    /// Reads from a channel with name `channel_name`
    ///
    /// Each channel instance is a reference to the same underlying data but keeps track
    /// of a separate position so calling this on different channel sets will refer to
    /// different positions in the data stream.
    pub async fn read_channel(
        &self,
        channel_name: &str,
        size: usize,
    ) -> Option<ChannelDataView<'_>> {
        futures::future::OptionFuture::from(
            self.channel(channel_name).map(|channel| channel.read(size)),
        )
        .await
    }

    /// Tells whether this [`ChannelSet`] has a [`Channel`] with `channel_name`
    pub fn has_channel(&self, channel_name: &str) -> bool {
        self.0.contains_key(channel_name)
    }

    /// Validate the [`ChannelSet`] against a set of specs
    ///
    /// `required` indicates which [`Channel`]s are required and the expected types of them
    /// `optional` indicates optional [`Channel`]s and their types
    /// Returns an empty [`Result`] on success and a list of [`ChannelSetValidationError`]
    /// if anything in this [`ChannelSet`] violates the validation expectations.
    pub async fn validate(
        &self,
        required: &HashMap<String, ProtoChannelSpec>,
        optional: Option<&HashMap<String, ProtoChannelSpec>>,
    ) -> Result<(), Vec<ChannelSetValidationError>> {
        let err: Vec<ChannelSetValidationError> = futures::stream::iter(
            required
                .iter()
                .map(|(name, channel_spec)| {
                    self.0
                        .get(name)
                        .map(|channel| (name, channel, channel_spec))
                        .ok_or_else(|| {
                            ChannelSetValidationError::RequiredChannelMissing(name.clone())
                        })
                })
                .chain(optional.iter().flat_map(|o| {
                    o.iter().filter_map(|(name, channel_spec)| {
                        self.0.get(name).map(|c| Ok((name, c, channel_spec)))
                    })
                }))
                .chain(self.0.keys().filter_map(|k| {
                    if required.contains_key(k) || optional.map_or(false, |opt| opt.contains_key(k))
                    {
                        None
                    } else {
                        Some(Err(ChannelSetValidationError::UnexpectedChannel(k.clone())))
                    }
                })),
        )
        .filter_map(|r| async {
            match r {
                Ok((name, channel, channel_spec)) => {
                    if channel.type_matches(channel_spec).await {
                        None
                    } else {
                        Some(ChannelSetValidationError::MismatchedChannelType {
                            channel_name: name.clone(),
                            expected: ChannelSpecType(ProtoChannelType::from_i32(
                                channel_spec.r#type,
                            ))
                            .to_string(),
                            got: channel.data.read().await.to_string(),
                        })
                    }
                }
                Err(e) => Some(e),
            }
        })
        .collect()
        .await;

        if err.is_empty() {
            Ok(())
        } else {
            Err(err)
        }
    }

    /// Obtain a reference to a [`Channel`] with `channel_name`
    ///
    /// Returns [`None`] if the [`Channel`] does not exist in the [`ChannelSet`]
    pub fn channel(&self, channel_name: &str) -> Option<&Channel> {
        self.0.get(channel_name)
    }

    /// Merge this [`ChannelSet`] with the [`ChannelSet`] denoted by `other`.
    ///
    /// If a [`Channel`] exists in both sets, the [`Channel`]s from `other` will take precedence.
    pub async fn merge(&self, other: &ChannelSet) -> ChannelSet<T> {
        // Take everything from both but right will overwrite conflicts with left.
        // It applies left before right and since right is last it overwrites.
        self.merge_with(other, |from| async move { Some(from.to_owned().await) })
            .await
    }

    /// Merge this [`ChannelSet`] with the [`ChannelSet`] denoted by `other`, using `f`
    ///
    /// The function `f` will be called for each item in both sets, with an instance of
    /// [`FromChannelSet`], indicating which set the entry came from. `f` then returns a
    /// future of an owned tuple of `([String], [Channel])`, wrapped in an option to
    /// inidicate if this entry should be included in the final set or not.
    pub async fn merge_with<'f, F, Fut>(&'f self, other: &'f ChannelSet, f: F) -> ChannelSet<T>
    where
        Fut: Future<Output = Option<(String, Channel)>> + 'f,
        F: FnMut(FromChannelSet<'f>) -> Fut,
    {
        ChannelSet(
            futures::stream::iter(
                self.0
                    .iter()
                    .map(|(name, channel)| FromChannelSet::Left(name.as_str(), channel))
                    .chain(
                        other
                            .0
                            .iter()
                            .map(|(name, channel)| FromChannelSet::Right(name.as_str(), channel)),
                    ),
            )
            .filter_map(f)
            .collect()
            .await,
            T::new(),
        )
    }

    /// Creates a new [`ChannelSet`] with reader capabilities from this [`ChannelSet`].
    pub async fn reader(&self) -> ChannelSet<ChannelReader> {
        ChannelSet(
            futures::stream::iter(self.0.iter())
                .then(|(name, channel)| channel.clone().map(|chan| (name.to_owned(), chan)))
                .collect()
                .await,
            ChannelReader(),
        )
    }
}

impl ChannelSet<ChannelWriter> {
    /// Append data to a [`Channel`]
    ///
    /// `channel_name` is the channel to append data to and `value` is the values to
    /// append. This method will return an error if the type that already exists for the
    /// [`Channel`] is different from the type of `value`.
    pub async fn append_channel<T>(
        &mut self,
        channel_name: &str,
        value: T,
    ) -> Result<(), ChannelSetError>
    where
        T: ChannelData,
    {
        futures::future::ready(
            self.0
                .get_mut(channel_name)
                .ok_or_else(|| ChannelSetError::NonExistingChannel(channel_name.to_owned())),
        )
        .and_then(|channel| {
            channel
                .append(value)
                .map_err(|e| ChannelSetError::FailedToAppendOutput(channel_name.to_owned(), e))
        })
        .await
    }

    /// Close a channel
    ///
    /// Closing a channel means that no more data can be written to it.
    ///
    /// `channel_name` is the name of the output channel to close.
    ///
    /// If a channel with name `channel_name` does not exist, this function does nothing.
    pub fn close_channel(&mut self, channel_name: &str) {
        if let Some(c) = self.0.get_mut(channel_name) {
            c.close();
        };
    }

    /// Obtain a mutable reference to a [`Channel`] with `channel_name`
    ///
    /// Returns [`None`] if the [`Channel`] does not exist in the [`ChannelSet`]
    pub fn channel_mut(&mut self, channel_name: &str) -> Option<&mut Channel> {
        self.0.get_mut(channel_name)
    }
}

/// Indicates which [`ChannelSet`] an entry originated from
///
/// Used when merging two [`ChannelSet`]s
pub enum FromChannelSet<'a> {
    Left(&'a str, &'a Channel),
    Right(&'a str, &'a Channel),
}

impl FromChannelSet<'_> {
    /// Create an owned [`ChannelSet`] entry, cloning the contained values
    pub async fn to_owned(&self) -> (String, Channel) {
        match *self {
            FromChannelSet::Left(name, channel) => (name.to_owned(), channel.clone().await),
            FromChannelSet::Right(name, channel) => (name.to_owned(), channel.clone().await),
        }
    }

    /// True if the [`ChannelSet`] entry came from the left side of the merge operation
    pub fn is_from_left(&self) -> bool {
        match self {
            FromChannelSet::Left(_, _) => true,
            FromChannelSet::Right(_, _) => false,
        }
    }

    /// True if the [`ChannelSet`] entry came from the right side of the merge operation
    pub fn is_from_right(&self) -> bool {
        !self.is_from_left()
    }

    /// Obtain a reference to the name of the contained [`ChannelSet`] entry
    pub fn name(&self) -> &str {
        match *self {
            FromChannelSet::Left(name, _) => name,
            FromChannelSet::Right(name, _) => name,
        }
    }

    /// Obtain a reference to the [`Channel`] of the contained [`ChannelSet`] entry
    pub fn channel(&self) -> &Channel {
        match *self {
            FromChannelSet::Left(_, channel) => channel,
            FromChannelSet::Right(_, channel) => channel,
        }
    }
}

impl From<&HashMap<String, ProtoChannelSpec>> for ChannelSet {
    /// Create a [`ChannelSet`] with writing capabilities from a channel spec map
    fn from(channels: &HashMap<String, ProtoChannelSpec>) -> Self {
        Self(
            channels
                .iter()
                .map(|(name, spec)| (name.to_owned(), spec.into()))
                .collect(),
            ChannelWriter(),
        )
    }
}

impl From<ProtoChannelSet> for ChannelSet {
    /// Create a [`ChannelSet`] with writing capabilities from a protobuf channel set.
    fn from(channel_set: ProtoChannelSet) -> Self {
        Self(
            channel_set
                .channels
                .into_iter()
                .map(|(name, proto_channel)| (name, proto_channel.into()))
                .collect(),
            ChannelWriter(),
        )
    }
}

/// Trait implemented for anything that can be turned into typed [`Channel`] data
pub trait ChannelData {
    fn to_channel_data(self) -> TypedChannelData;
}

macro_rules! impl_to_data {
    ($rust_type:ty, $enum_member:expr) => {
        impl ChannelData for Vec<$rust_type> {
            fn to_channel_data(self) -> TypedChannelData {
                $enum_member(self)
            }
        }

        impl ChannelData for &[$rust_type] {
            fn to_channel_data(self) -> TypedChannelData {
                $enum_member(self.to_vec())
            }
        }
    };
}

macro_rules! impl_to_data_with_conversion {
    ($rust_type:ty, $enum_member:expr) => {
        impl ChannelData for Vec<$rust_type> {
            fn to_channel_data(self) -> TypedChannelData {
                $enum_member(self.into_iter().map(|v| v.into()).collect())
            }
        }

        impl ChannelData for &[$rust_type] {
            fn to_channel_data(self) -> TypedChannelData {
                $enum_member(self.iter().map(|v| (*v).into()).collect())
            }
        }
    };
}

impl_to_data!(String, TypedChannelData::Strings);
impl_to_data_with_conversion!(&str, TypedChannelData::Strings);

impl_to_data!(i64, TypedChannelData::Integers);
impl_to_data_with_conversion!(i32, TypedChannelData::Integers);
impl_to_data_with_conversion!(u32, TypedChannelData::Integers);

impl_to_data_with_conversion!(i16, TypedChannelData::Integers);
impl_to_data_with_conversion!(u16, TypedChannelData::Integers);

impl_to_data_with_conversion!(i8, TypedChannelData::Integers);

impl_to_data!(f64, TypedChannelData::Floats);
impl_to_data_with_conversion!(f32, TypedChannelData::Floats);

impl_to_data!(bool, TypedChannelData::Booleans);
impl_to_data!(u8, TypedChannelData::Bytes);

impl ChannelData for () {
    fn to_channel_data(self) -> TypedChannelData {
        TypedChannelData::Null(self)
    }
}

/// [`Channel`] data with associated type information
pub enum TypedChannelData {
    Strings(Vec<String>),
    Integers(Vec<i64>),
    Floats(Vec<f64>),
    Booleans(Vec<bool>),
    Bytes(Vec<u8>),
    Null(()),
}

impl TypedChannelData {
    /// Get the length of the typed data
    pub fn len(&self) -> usize {
        match self {
            TypedChannelData::Strings(s) => s.len(),
            TypedChannelData::Integers(i) => i.len(),
            TypedChannelData::Floats(f) => f.len(),
            TypedChannelData::Booleans(b) => b.len(),
            TypedChannelData::Bytes(b) => b.len(),
            TypedChannelData::Null(_) => 0,
        }
    }

    /// True if there is no data
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Display for TypedChannelData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                TypedChannelData::Strings(_) => "strings",
                TypedChannelData::Integers(_) => "integers",
                TypedChannelData::Floats(_) => "floats",
                TypedChannelData::Booleans(_) => "booleans",
                TypedChannelData::Bytes(_) => "bytes",
                TypedChannelData::Null(_) => "null",
            }
        )
    }
}

impl From<&ProtoChannelSpec> for TypedChannelData {
    fn from(spec: &ProtoChannelSpec) -> Self {
        match ProtoChannelType::from_i32(spec.r#type) {
            Some(ProtoChannelType::String) => TypedChannelData::Strings(Vec::new()),
            Some(ProtoChannelType::Int) => TypedChannelData::Integers(Vec::new()),
            Some(ProtoChannelType::Float) => TypedChannelData::Floats(Vec::new()),
            Some(ProtoChannelType::Bool) => TypedChannelData::Booleans(Vec::new()),
            Some(ProtoChannelType::Bytes) => TypedChannelData::Bytes(Vec::new()),
            None => TypedChannelData::Null(()),
        }
    }
}

struct WakerSlot(Mutex<Option<Waker>>);

impl WakerSlot {
    fn new() -> Self {
        Self(Mutex::new(None))
    }

    async fn wake(&self) {
        if let Some(waker) = self.0.lock().await.take() {
            waker.wake()
        }
    }
}

impl Deref for WakerSlot {
    type Target = Mutex<Option<Waker>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Error, Debug)]
pub enum ChannelError {
    #[error("Type mismatch: Expected \"{0}\", got \"{1}\"")]
    MismatchedTypes(String, String),

    #[error("Channel is closed.")]
    ChannelClosed,
}

/// Type representing input or output data for a function
///
/// A [`Channel`] also tracks state for iterating over the data and gives the ability to
/// append data to the [`Channel`], given that you have a mutable copy of it.
pub struct Channel {
    closed: Arc<AtomicBool>,
    data: Arc<RwLock<TypedChannelData>>,
    wakers_to_notify: Arc<Mutex<Vec<Arc<WakerSlot>>>>,
    pos: AtomicUsize,
    waker: Arc<WakerSlot>,
    seek_pos: Option<std::io::SeekFrom>,
}

impl Channel {
    /// Create a new [`Channel`] with `data`
    pub fn new(data: TypedChannelData) -> Self {
        let waker = Arc::new(WakerSlot::new());
        Self {
            wakers_to_notify: Arc::new(Mutex::new(vec![Arc::clone(&waker)])),
            closed: Arc::new(false.into()),
            data: Arc::new(RwLock::new(data)),
            pos: AtomicUsize::new(0),
            waker,
            seek_pos: None,
        }
    }

    /// Create a new [`Channel`] that refers to the same data but tracks a new position
    async fn clone(&self) -> Self {
        let waker = Arc::new(WakerSlot::new());
        self.wakers_to_notify.lock().await.push(Arc::clone(&waker));
        Self {
            closed: Arc::clone(&self.closed),
            data: Arc::clone(&self.data),
            wakers_to_notify: Arc::clone(&self.wakers_to_notify),
            pos: AtomicUsize::new(self.pos.load(Ordering::Relaxed)),
            waker,
            seek_pos: self.seek_pos,
        }
    }

    /// Close this [`Channel`] for further writing
    pub fn close(&mut self) {
        self.closed.store(true, Ordering::SeqCst)
    }

    /// Read data from the [`Channel`]
    ///
    /// Data is read in a chunk of `size` elements and given back as a [`ChannelDataView`]
    /// that can be used to either obtain a Rust-typed slice with the data or a
    /// [`TypedChannelDataRef`] containing type information along with the data.
    pub async fn read(&self, size: usize) -> ChannelDataView<'_> {
        let lock_guard = self.data.read().await;
        let (start, end) = ChannelDataReadRequest {
            data_len: lock_guard.len(),
            waker: self.waker.lock().await,
            requested_size: size,
            closed: Arc::clone(&self.closed),
            pos: &self.pos,
        }
        .await;

        ChannelDataView {
            lock_guard,
            start,
            end,
        }
    }

    /// Append data to the [`Channel`]
    ///
    /// This will add `value` to the end of the [`Channel`]. For a successful append this
    /// returns an empty [`Result`]. On a type mismatch between `value` and the data
    /// already in the [`Channel`] or if the [`Channel`] is closed, it will return a
    /// [`ChannelError`].
    pub async fn append<T>(&mut self, value: T) -> Result<(), ChannelError>
    where
        T: ChannelData,
    {
        if self.closed.load(Ordering::SeqCst) {
            return Err(ChannelError::ChannelClosed);
        }

        match (self.data.write().await.deref_mut(), value.to_channel_data()) {
            (TypedChannelData::Strings(existsing_data), TypedChannelData::Strings(new_data)) => {
                existsing_data.extend(new_data);
                Ok(())
            }
            (TypedChannelData::Integers(existsing_data), TypedChannelData::Integers(new_data)) => {
                existsing_data.extend(new_data);
                Ok(())
            }
            (TypedChannelData::Floats(existsing_data), TypedChannelData::Floats(new_data)) => {
                existsing_data.extend(new_data);
                Ok(())
            }
            (TypedChannelData::Booleans(existsing_data), TypedChannelData::Booleans(new_data)) => {
                existsing_data.extend(new_data);
                Ok(())
            }
            (TypedChannelData::Bytes(existsing_data), TypedChannelData::Bytes(new_data)) => {
                existsing_data.extend(new_data);
                Ok(())
            }
            (TypedChannelData::Null(_), TypedChannelData::Null(_)) => Ok(()),
            (existing_data, new_data) => Err(ChannelError::MismatchedTypes(
                existing_data.to_string(),
                new_data.to_string(),
            )),
        }?;

        futures::stream::iter(self.wakers_to_notify.lock().await.iter())
            .for_each(|waker_slot| waker_slot.wake())
            .await;
        Ok(())
    }

    async fn type_matches(&self, spec: &ProtoChannelSpec) -> bool {
        match (
            self.data.read().await.deref(),
            ProtoChannelType::from_i32(spec.r#type),
        ) {
            (TypedChannelData::Strings(_), Some(ProtoChannelType::String))
            | (TypedChannelData::Integers(_), Some(ProtoChannelType::Int))
            | (TypedChannelData::Floats(_), Some(ProtoChannelType::Float))
            | (TypedChannelData::Booleans(_), Some(ProtoChannelType::Bool))
            | (TypedChannelData::Bytes(_), Some(ProtoChannelType::Bytes))
            | (TypedChannelData::Null(_), None) => true,
            (_, _) => false,
        }
    }
}

impl<T: ChannelData> From<T> for Channel {
    fn from(c: T) -> Self {
        Self::new(c.to_channel_data())
    }
}

impl From<ProtoChannel> for Channel {
    fn from(c: ProtoChannel) -> Self {
        Self::new(match c.value {
            Some(ProtoValue::Strings(s)) => TypedChannelData::Strings(s.values),
            Some(ProtoValue::Integers(i)) => TypedChannelData::Integers(i.values),
            Some(ProtoValue::Booleans(b)) => TypedChannelData::Booleans(b.values),
            Some(ProtoValue::Floats(f)) => TypedChannelData::Floats(f.values),
            Some(ProtoValue::Bytes(b)) => TypedChannelData::Bytes(b.values),
            None => TypedChannelData::Null(()),
        })
    }
}

impl From<&ProtoChannelSpec> for Channel {
    fn from(spec: &ProtoChannelSpec) -> Self {
        Self::new(spec.into())
    }
}

impl Debug for Channel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Channel")
            .field("data_ptr", &Arc::as_ptr(&self.data))
            .field("closed", &self.closed)
            .finish()
    }
}

impl PartialEq for Channel {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.data, &other.data)
    }
}

impl AsyncSeek for Channel {
    fn start_seek(mut self: Pin<&mut Self>, pos: std::io::SeekFrom) -> std::io::Result<()> {
        if self.seek_pos.is_none() {
            self.seek_pos.replace(pos);
            Ok(())
        } else {
            Err(std::io::Error::from(std::io::ErrorKind::Other))
        }
    }

    fn poll_complete(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<u64>> {
        let seek_pos = self.seek_pos.take();
        let mut current_position = self.pos.load(Ordering::Relaxed);

        // If no one changed the seek pos there is no point in trying to set current position.
        if seek_pos.is_none() {
            return Poll::Ready(Ok(current_position as u64));
        }

        match Pin::new(&mut self.data.read().boxed()).poll(cx) {
            Poll::Ready(guard) => {
                let len = guard.len();

                loop {
                    let new_pos = match seek_pos {
                        Some(std::io::SeekFrom::Start(offset)) => (offset as usize).clamp(0, len),
                        Some(std::io::SeekFrom::End(offset)) => {
                            (len as i64 + offset).clamp(0, len as i64) as usize
                        }
                        Some(std::io::SeekFrom::Current(offset)) => {
                            (current_position as i64 + offset).clamp(0, len as i64) as usize
                        }
                        None => current_position,
                    };

                    match self.pos.compare_exchange(
                        current_position,
                        new_pos,
                        Ordering::Acquire,
                        Ordering::Relaxed,
                    ) {
                        Ok(current) => {
                            current_position = current;
                            break;
                        }
                        Err(new_current) => {
                            current_position = new_current;
                        }
                    };
                }
                Poll::Ready(Ok(current_position as u64))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

/// A view on a slice of data from a [`Channel`]
pub struct ChannelDataView<'a> {
    lock_guard: RwLockReadGuard<'a, TypedChannelData>,
    start: usize,
    end: Option<usize>,
}

impl ChannelDataView<'_> {
    /// Retrieve the data in this view as data with associated type info
    pub fn as_typed_data(&self) -> Option<TypedChannelDataRef<'_>> {
        match self.lock_guard.deref() {
            TypedChannelData::Strings(s) => Some(TypedChannelDataRef::Strings(
                &s[self.start..self.end.unwrap_or(s.len())],
            )),
            TypedChannelData::Integers(i) => Some(TypedChannelDataRef::Integers(
                &i[self.start..self.end.unwrap_or(i.len())],
            )),
            TypedChannelData::Floats(f) => Some(TypedChannelDataRef::Floats(
                &f[self.start..self.end.unwrap_or(f.len())],
            )),
            TypedChannelData::Booleans(b) => Some(TypedChannelDataRef::Booleans(
                &b[self.start..self.end.unwrap_or(b.len())],
            )),
            TypedChannelData::Bytes(b) => Some(TypedChannelDataRef::Bytes(
                &b[self.start..self.end.unwrap_or(b.len())],
            )),
            TypedChannelData::Null(_) => None,
        }
    }

    /// Retrieve the data in this view as a slice of type `T`
    ///
    /// Will fail with a [`ChannelError`] if the data cannot be interpreted as `T`
    pub fn as_slice<'a, T: TypedDataAsSlice<'a>>(&'a self) -> Result<&'a [T], ChannelError> {
        T::as_slice(self.lock_guard.deref(), self.start, self.end)
    }

    /// Retrieve the len of the data in this view
    pub fn len(&self) -> usize {
        self.end.unwrap_or_else(|| self.lock_guard.deref().len()) - self.start
    }

    /// Returns true if there is data in this view
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Trait for converting a typed data reference to a Rust type
pub trait TypedDataAsSlice<'a> {
    fn as_slice(
        cd: &'a TypedChannelData,
        start: usize,
        end: Option<usize>,
    ) -> Result<&'a [Self], ChannelError>
    where
        Self: Sized;
}

macro_rules! as_slice_impl {
    ($ref_type:ty, $expected_type:path) => {
        impl<'a> TypedDataAsSlice<'a> for $ref_type {
            fn as_slice(
                cd: &'a TypedChannelData,
                start: usize,
                end: Option<usize>,
            ) -> Result<&'a [$ref_type], ChannelError> {
                let len = cd.len();
                if let $expected_type(v) = cd {
                    Ok(&v[start..end.unwrap_or(len)])
                } else {
                    Err(ChannelError::MismatchedTypes(
                        String::from(stringify!($ref_type)),
                        cd.to_string(),
                    ))
                }
            }
        }
    };
}

as_slice_impl!(String, TypedChannelData::Strings);
as_slice_impl!(i64, TypedChannelData::Integers);
as_slice_impl!(f64, TypedChannelData::Floats);
as_slice_impl!(bool, TypedChannelData::Booleans);
as_slice_impl!(u8, TypedChannelData::Bytes);

/// A view on a slice of [`Channel`] data with type information.
pub enum TypedChannelDataRef<'a> {
    Strings(&'a [String]),
    Integers(&'a [i64]),
    Floats(&'a [f64]),
    Booleans(&'a [bool]),
    Bytes(&'a [u8]),
}

impl TypedChannelDataRef<'_> {
    /// Owning the data converting into a [`TypedChannelData`] by copying the referenced data.
    pub fn to_owned(&self) -> TypedChannelData {
        match self {
            TypedChannelDataRef::Strings(s) => TypedChannelData::Strings(s.to_vec()),
            TypedChannelDataRef::Integers(i) => TypedChannelData::Integers(i.to_vec()),
            TypedChannelDataRef::Floats(f) => TypedChannelData::Floats(f.to_vec()),
            TypedChannelDataRef::Booleans(b) => TypedChannelData::Booleans(b.to_vec()),
            TypedChannelDataRef::Bytes(b) => TypedChannelData::Bytes(b.to_vec()),
        }
    }
}

struct ChannelDataReadRequest<'a> {
    data_len: usize,
    waker: MutexGuard<'a, Option<Waker>>,
    requested_size: usize,
    closed: Arc<AtomicBool>,
    pos: &'a AtomicUsize,
}

impl<'a> Future for ChannelDataReadRequest<'a> {
    type Output = (usize, Option<usize>);

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut original_pos = self.pos.load(Ordering::Relaxed);
        let mut new_pos;
        loop {
            new_pos = (original_pos + self.requested_size).clamp(0, self.data_len);
            match self.pos.compare_exchange(
                original_pos,
                new_pos,
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(original) => {
                    original_pos = original;
                    break;
                }
                Err(new_original) => {
                    original_pos = new_original;
                }
            }
        }

        if (new_pos - original_pos) >= self.requested_size {
            Poll::Ready((original_pos, Some(original_pos + self.requested_size)))
        } else if self.closed.load(Ordering::SeqCst) {
            Poll::Ready((original_pos, None))
        } else {
            self.waker.replace(cx.waker().clone());
            Poll::Pending
        }
    }
}

struct ChannelSpecType(Option<ProtoChannelType>);
impl Display for ChannelSpecType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self.0 {
                Some(ProtoChannelType::String) => String::from("string"),
                Some(ProtoChannelType::Int) => String::from("integer"),
                Some(ProtoChannelType::Bool) => String::from("boolean"),
                Some(ProtoChannelType::Float) => String::from("float"),
                Some(ProtoChannelType::Bytes) => String::from("bytes"),
                None => format!("Unknown type with discriminator {}", self),
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    use firm_types::channel_specs;
    use tokio::{io::AsyncSeekExt, sync::oneshot, time::timeout};

    #[tokio::test]
    async fn test_channel_set() {
        let channel_set = ChannelSet::from(&HashMap::new());

        assert!(
            channel_set.0.is_empty(),
            "Expected channelset with default to have an empty channel set"
        );

        let (required, optional) = channel_specs!(
            {
                "grape" =>
                    ProtoChannelSpec {
                        r#type: ProtoChannelType::String as i32,
                        description: String::from("How many grapes you need."),
                    },
                "melon" =>
                    ProtoChannelSpec {
                        r#type: ProtoChannelType::Int as i32,
                        description: String::from("How many melons you need."),
                    },
                "kiwi" =>
                    ProtoChannelSpec {
                        r#type: ProtoChannelType::Bool as i32,
                        description: String::from("How many kiwis you need."),
                    }
            }
        );

        // Channel set must contain all channels it was created from.
        let mut channel_set = ChannelSet::from(&required);
        assert!(
            channel_set.has_channel("grape"),
            "Expected grape to be in channel set."
        );

        assert!(
            channel_set.has_channel("melon"),
            "Expected channel set to contain melon"
        );

        assert!(
            channel_set.has_channel("kiwi"),
            "Expected channel set to contain kiwi"
        );

        // Data insertion
        let res = channel_set
            .append_channel("grape", ["mega", "brain"].as_slice())
            .await;
        assert!(
            res.is_ok(),
            "Expected to be able to append string data to channel grape"
        );

        let res = channel_set
            .append_channel("melon", [1, 2, 3, 4].as_slice())
            .await;
        assert!(
            res.is_ok(),
            "Expected to be able to append int data to channel melon"
        );

        let res = channel_set
            .append_channel("kiwi", [false, true, true, false].as_slice())
            .await;
        assert!(
            res.is_ok(),
            "Expected to be able to append boolean data to channel kiwi"
        );

        // Check that we can't append floats to a string channel.
        let res = channel_set
            .append_channel("grape", [0.1, 0.2, 0.3, 0.4].as_slice())
            .await;
        assert!(
            res.is_err(),
            "Expected not to be able to append float data to a string channel grape"
        );

        // Test behaviour of closed channel.
        channel_set.close_channel("grape");

        let res = channel_set
            .append_channel("grape", ["late", "data"].as_slice())
            .await;
        assert!(
            res.is_err(),
            "Did not expect to be able to write to closed channel grape."
        );

        // Channel spec validation.
        let res = channel_set.validate(&required, optional.as_ref()).await;
        assert!(res.is_ok(), "Expected validation to pass.");

        let (bad_required, bad_optional) = channel_specs!(
            {
                "grape" =>
                    ProtoChannelSpec {
                        r#type: ProtoChannelType::Int as i32, // Changed this type to be wrong.
                        description: String::from("How many grapes you need."),
                    },
                "melon" =>
                    ProtoChannelSpec {
                        r#type: ProtoChannelType::Int as i32,
                        description: String::from("How many melons you need."),
                    },
                "kiwi" =>
                    ProtoChannelSpec {
                        r#type: ProtoChannelType::Bool as i32,
                        description: String::from("How many kiwis you need."),
                    }
            }
        );

        let res = channel_set
            .validate(&bad_required, bad_optional.as_ref())
            .await;
        assert!(
            res.is_err(),
            "Expected validation to be wrong due to grape having wrong type."
        );

        // Getting channels
        let channel = channel_set.channel("grape");
        assert!(channel.is_some(), "Expected grape channel to exist");

        // Data is as expected
        let channel = channel.unwrap();
        assert!(
            channel.closed.load(Ordering::SeqCst),
            "Expected grape channel to be closed"
        );

        let channel = channel_set.channel_mut("grape");
        assert!(channel.is_some(), "Expected grape mut channel to exist");
    }

    #[tokio::test]
    async fn test_channel_ref() {
        let (required, _) = channel_specs!(
            {
                "ananas" =>
                    ProtoChannelSpec {
                        r#type: ProtoChannelType::Int as i32,
                        description: String::from("All the ananas you need."),
                    }
            }
        );

        let mut channel_set = ChannelSet::from(&required);
        assert!(
            channel_set.has_channel("ananas"),
            "Expected ananas to be in ref channel set."
        );

        // Write to the channel
        let _ = channel_set
            .append_channel("ananas", &[1i64, 3, 3, 7] as &[i64])
            .await;

        let _ = channel_set
            .append_channel("ananas", &[4, 5, 6, 7] as &[i32])
            .await;

        let mut channel_ref = channel_set.channel("ananas").unwrap().clone().await;

        // Two partial reads
        match channel_ref.read(2).await.as_typed_data() {
            Some(TypedChannelDataRef::Integers(i)) => {
                assert_eq!(i, [1, 3], "Expected to get first 2 integers")
            }
            _ => panic!("Read unexpected type. Expected it to be integers."),
        };

        channel_set.close_channel("ananas");

        match channel_ref.read(20).await.as_slice::<i64>() {
            Ok(i) => {
                assert_eq!(i, [3, 7, 4, 5, 6, 7], "Expected to get all integers")
            }
            _ => panic!("Read unexpected type. Expected it to be integers."),
        };

        // All data available after rewind
        assert!(
            channel_ref.rewind().await.is_ok(),
            "Expected rewind to not explode."
        );

        match channel_ref.read(20).await.as_slice::<i64>() {
            Ok(i) => {
                assert_eq!(
                    i,
                    [1, 3, 3, 7, 4, 5, 6, 7],
                    "Expected to get all integers after rewind"
                )
            }
            _ => panic!("Read unexpected type. Expected it to be integers."),
        };

        // Read wrong type.
        assert!(
            channel_ref.rewind().await.is_ok(),
            "Expected rewind to not explode."
        );

        assert!(
            channel_ref.read(20).await.as_slice::<String>().is_err(),
            "Expected getting slice as wrong type would result in error"
        );

        // Read from middle of stream
        assert!(
            channel_ref.seek(std::io::SeekFrom::End(-2)).await.is_ok(),
            "Expected to be able to seek"
        );

        match channel_ref.read(2).await.as_slice::<i64>() {
            Ok(i) => {
                assert_eq!(i, [6, 7], "Expected to get all integers after seek")
            }
            _ => panic!("Read unexpected type. Expected it to be integers."),
        };

        // start_seek behaviour
        assert!(
            Pin::new(&mut channel_ref)
                .start_seek(std::io::SeekFrom::Start(2))
                .is_ok(),
            "Expected to be able to seek."
        );

        assert!(
            Pin::new(&mut channel_ref)
                .start_seek(std::io::SeekFrom::Start(2))
                .is_err(),
            "Expected to get error when doing two start_seeks with no poll in between."
        );
    }

    #[tokio::test]
    async fn test_channel_set_merge_read() {
        // Setup for outputs for function A
        let (required_a, _) = channel_specs!(
            {
                "kumquat" =>
                    ProtoChannelSpec {
                        r#type: ProtoChannelType::Int as i32,
                        description: String::from("All the kumquat you need."),
                    }
            }
        );

        let mut channel_set_a = ChannelSet::from(&required_a);

        // Setup for outputs for function B
        let (required_b, _) = channel_specs!(
            {
                "salak" =>
                    ProtoChannelSpec {
                        r#type: ProtoChannelType::Int as i32,
                        description: String::from("All the salak you need."),
                    },
                "rambutan" =>
                    ProtoChannelSpec {
                        r#type: ProtoChannelType::Int as i32,
                        description: String::from("All the rambutan you need."),
                    }
            }
        );

        let mut channel_set_b = ChannelSet::from(&required_b);

        // Merge both sets
        let ref_channel_set_merged = channel_set_a.merge(&channel_set_b).await;

        // Write some data to all channels
        channel_set_a
            .append_channel("kumquat", [1i64, 2, 3].as_slice())
            .await
            .expect("Could not append to kumquat");

        channel_set_b
            .append_channel("salak", [4i64, 5, 6].as_slice())
            .await
            .expect("Could not append to salak");

        channel_set_b
            .append_channel("rambutan", [7i64, 8, 9].as_slice())
            .await
            .expect("Could not append to rambutan");

        // Read from merged channel
        match ref_channel_set_merged
            .read_channel("kumquat", 2)
            .await
            .as_ref()
            .map(|data| data.as_slice::<i64>())
        {
            Some(Ok(i)) => {
                assert_eq!(i, [1, 2], "Expected to be able to read from kumquat.")
            }
            _ => panic!("Kumquat did not exist or was the wrong type"),
        };

        match ref_channel_set_merged
            .read_channel("salak", 2)
            .await
            .as_ref()
            .map(|data| data.as_slice::<i64>())
        {
            Some(Ok(i)) => {
                assert_eq!(i, [4, 5], "Expected to be able to read from salak.")
            }
            _ => panic!("Salak did not exist or was the wrong type"),
        };

        match ref_channel_set_merged
            .read_channel("rambutan", 2)
            .await
            .as_ref()
            .map(|data| data.as_slice::<i64>())
        {
            Some(Ok(i)) => {
                assert_eq!(i, [7, 8], "Expected to be able to read from rambutan.")
            }
            _ => panic!("Rambutan did not exist or was the wrong type."),
        };
    }

    #[tokio::test]
    async fn test_channel_set_merging() {
        // Setup for outputs for function A
        let (required_a, _) = channel_specs!(
            {
                "jaboticaba" =>
                    ProtoChannelSpec {
                        r#type: ProtoChannelType::Int as i32,
                        description: String::from("All the jaboticaba you need."),
                    }
            }
        );

        let channel_set_a = ChannelSet::from(&required_a);

        // Setup for outputs for function B
        let (required_b, _) = channel_specs!(
            {
                "lichi" =>
                    ProtoChannelSpec {
                        r#type: ProtoChannelType::Int as i32,
                        description: String::from("All the lichi you need."),
                    },
                "achiote" =>
                    ProtoChannelSpec {
                        r#type: ProtoChannelType::Int as i32,
                        description: String::from("All the achiote you need."),
                    }
            }
        );

        let channel_set_b = ChannelSet::from(&required_b);

        // Merging stuff
        assert_eq!(
            channel_set_a.merge(&channel_set_b).await,
            channel_set_b.merge(&channel_set_a).await,
            "Expected ref sets resulting from merge to point to same data"
        );
    }

    #[tokio::test]
    async fn test_channel_multi_read() {
        let (required, _) = channel_specs!(
            {
                "mangosteen" =>
                    ProtoChannelSpec {
                        r#type: ProtoChannelType::Int as i32,
                        description: String::from("All the mangosteen you need."),
                    }
            }
        );

        let mut write_channel_set = ChannelSet::from(&required);
        let (write_last, can_write_last) = oneshot::channel();
        let read_channel_set_a = write_channel_set.reader().await;
        let read_channel_set_b = read_channel_set_a.reader().await; // yields the same type of reader.

        let write_task = tokio::spawn(async move {
            let mangosteen = write_channel_set
                .channel_mut("mangosteen")
                .expect("Expected mangosteen channel to exist");
            mangosteen.append(vec![1i32, 2]).await.unwrap();
            mangosteen.append(vec![3i32, 4]).await.unwrap();
            let _ = can_write_last.await;
            mangosteen.append(vec![5i32, 6]).await.unwrap();
            mangosteen.close();
        });

        assert!(
            read_channel_set_a
                .read_channel("mangosteen", 6)
                .now_or_never()
                .is_none(),
            "Expected reader_a to be pending."
        );

        write_last
            .send(())
            .expect("Could not send write_last message.");
        write_task
            .await
            .expect("Expected write_task to not explode");

        assert_eq!(
            timeout(
                Duration::from_secs(5),
                read_channel_set_a.read_channel("mangosteen", 6)
            )
            .await
            .expect("Timeout occured")
            .expect("mangonsteen channel did not exist.")
            .as_slice::<i64>()
            .unwrap(),
            [1, 2, 3, 4, 5, 6],
            "Expected reader_a to be done."
        );

        read_channel_set_b
            .read_channel("mangosteen", 3)
            .await
            .expect("Mangosteen channel did not exist");
        let read_channel_set_b1 = read_channel_set_b.reader().await;
        let read_channel_set_b2 = read_channel_set_b1.reader().await;

        let b1_data = read_channel_set_b1
            .read_channel("mangosteen", 3)
            .await
            .unwrap();

        let b2_data = read_channel_set_b2
            .read_channel("mangosteen", 3)
            .await
            .unwrap();

        assert_eq!(
            b1_data.as_slice::<i64>().unwrap(),
            [4, 5, 6],
            "Unexpected b1 data"
        );

        assert_eq!(
            b2_data.as_slice::<i64>().unwrap(),
            [4, 5, 6],
            "Unexpected b2 data"
        );
    }
}
