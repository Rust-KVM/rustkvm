//! A thin binding to the parts of Rockchip's `librknnrt` the engine uses,
//! loaded with `dlopen` so the app builds and starts without it.
//!
//! Besides init/run this needs `rknn_set_internal_mem` (the models of one
//! worker share one activation buffer), which needs the raw context handle.
//!
//! `rknn_dup_context` is not used: in librknnrt 2.5.0 duplicates of one
//! model running at the same time on different cores overwrite each other's
//! outputs.

use std::ffi::{c_char, c_int, c_void};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow, bail};
use rknpu2_sys::_rknn_core_mask::{RKNN_NPU_CORE_0, RKNN_NPU_CORE_1, RKNN_NPU_CORE_2};
use rknpu2_sys::_rknn_query_cmd::{
    RKNN_QUERY_INPUT_ATTR, RKNN_QUERY_MEM_SIZE, RKNN_QUERY_OUTPUT_ATTR, RKNN_QUERY_SDK_VERSION,
};
use rknpu2_sys::_rknn_tensor_format::{RKNN_TENSOR_NCHW, RKNN_TENSOR_NHWC};
use rknpu2_sys::_rknn_tensor_type::{RKNN_TENSOR_FLOAT16, RKNN_TENSOR_UINT8};
use rknpu2_sys::{
    RKNN_FLAG_INTERNAL_ALLOC_OUTSIDE, rknn_context, rknn_input, rknn_mem_size, rknn_output,
    rknn_sdk_version, rknn_tensor_attr, rknn_tensor_mem,
};

type InitFn = unsafe extern "C" fn(*mut rknn_context, *mut c_void, u32, u32, *mut c_void) -> c_int;
type DestroyFn = unsafe extern "C" fn(rknn_context) -> c_int;
type QueryFn = unsafe extern "C" fn(rknn_context, u32, *mut c_void, u32) -> c_int;
type InputsSetFn = unsafe extern "C" fn(rknn_context, u32, *mut rknn_input) -> c_int;
type CoreMaskFn = unsafe extern "C" fn(rknn_context, u32) -> c_int;
type RunFn = unsafe extern "C" fn(rknn_context, *mut c_void) -> c_int;
type OutputsGetFn = unsafe extern "C" fn(rknn_context, u32, *mut rknn_output, *mut c_void) -> c_int;
type CreateMemFn = unsafe extern "C" fn(rknn_context, u32) -> *mut rknn_tensor_mem;
type MemFn = unsafe extern "C" fn(rknn_context, *mut rknn_tensor_mem) -> c_int;

/// The loaded library and the entry points the engine calls.
pub struct Runtime {
    init: InitFn,
    destroy: DestroyFn,
    query: QueryFn,
    inputs_set: InputsSetFn,
    set_core_mask: CoreMaskFn,
    run: RunFn,
    outputs_get: OutputsGetFn,
    create_mem: CreateMemFn,
    destroy_mem: MemFn,
    set_internal_mem: MemFn,
    _library: libloading::Library,
}

fn symbol<T: Copy>(library: &libloading::Library, name: &[u8]) -> Result<T> {
    // SAFETY: every `T` used below is the C signature from rknn_api.h of the
    // symbol it is looked up with.
    unsafe { library.get::<T>(name) }
        .map(|s| *s)
        .with_context(|| format!("librknnrt has no {}", String::from_utf8_lossy(name)))
}

impl Runtime {
    pub fn load(path: &Path) -> Result<Arc<Self>> {
        // SAFETY: librknnrt runs no initialisers with preconditions on load.
        let library = unsafe { libloading::Library::new(path) }
            .with_context(|| format!("load {}", path.display()))?;
        Ok(Arc::new(Self {
            init: symbol(&library, b"rknn_init\0")?,
            destroy: symbol(&library, b"rknn_destroy\0")?,
            query: symbol(&library, b"rknn_query\0")?,
            inputs_set: symbol(&library, b"rknn_inputs_set\0")?,
            set_core_mask: symbol(&library, b"rknn_set_core_mask\0")?,
            run: symbol(&library, b"rknn_run\0")?,
            outputs_get: symbol(&library, b"rknn_outputs_get\0")?,
            create_mem: symbol(&library, b"rknn_create_mem\0")?,
            destroy_mem: symbol(&library, b"rknn_destroy_mem\0")?,
            set_internal_mem: symbol(&library, b"rknn_set_internal_mem\0")?,
            _library: library,
        }))
    }
}

fn check(ret: c_int, what: &str) -> Result<()> {
    if ret == 0 { Ok(()) } else { bail!("{what} failed ({ret})") }
}

fn c_string(chars: &[c_char]) -> String {
    let bytes: Vec<u8> = chars.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The NPU cores of the RK3588, as `rknn_set_core_mask` values.
pub const CORES: [u32; 3] = [RKNN_NPU_CORE_0, RKNN_NPU_CORE_1, RKNN_NPU_CORE_2];

/// An input or output tensor's shape and element type.
#[derive(Clone, Debug)]
pub struct Tensor {
    pub dims: Vec<u32>,
    pub elements: usize,
    pub is_nchw: bool,
    pub is_f16: bool,
}

impl From<&rknn_tensor_attr> for Tensor {
    fn from(attr: &rknn_tensor_attr) -> Self {
        let n = (attr.n_dims as usize).min(attr.dims.len());
        Self {
            dims: attr.dims[..n].to_vec(),
            elements: attr.n_elems as usize,
            is_nchw: attr.fmt == RKNN_TENSOR_NCHW,
            is_f16: attr.type_ == RKNN_TENSOR_FLOAT16,
        }
    }
}

/// One model context. Its activations live in memory given to it with
/// [`Context::set_internal_mem`] before the first run.
pub struct Context {
    rt: Arc<Runtime>,
    ctx: rknn_context,
}

// SAFETY: a context is a handle into librknnrt, which allows using it from
// any thread as long as calls on one context do not overlap; every method
// takes `&mut self` or is a read-only query.
unsafe impl Send for Context {}

impl Context {
    /// Loads a model whose activation memory the caller provides.
    pub fn load(rt: &Arc<Runtime>, model: &mut [u8]) -> Result<Self> {
        let mut ctx: rknn_context = 0;
        let size = u32::try_from(model.len()).context("model larger than 4 GiB")?;
        // SAFETY: `model` is valid for `size` bytes for the call; librknnrt
        // copies what it keeps.
        let ret = unsafe {
            (rt.init)(
                &mut ctx,
                model.as_mut_ptr().cast(),
                size,
                RKNN_FLAG_INTERNAL_ALLOC_OUTSIDE,
                std::ptr::null_mut(),
            )
        };
        check(ret, "rknn_init")?;
        Ok(Self { rt: rt.clone(), ctx })
    }

    fn query<T>(&self, cmd: u32, value: &mut T) -> Result<()> {
        let size = std::mem::size_of::<T>() as u32;
        // SAFETY: `value` is a valid `T`, the struct rknn_api.h pairs with `cmd`.
        check(
            unsafe { (self.rt.query)(self.ctx, cmd, (value as *mut T).cast(), size) },
            "rknn_query",
        )
    }

    fn attr(&self, cmd: u32) -> Result<Tensor> {
        // SAFETY: rknn_tensor_attr is plain data, valid when zeroed.
        let mut attr: rknn_tensor_attr = unsafe { std::mem::zeroed() };
        attr.index = 0;
        self.query(cmd, &mut attr)?;
        Ok(Tensor::from(&attr))
    }

    pub fn input(&self) -> Result<Tensor> {
        self.attr(RKNN_QUERY_INPUT_ATTR)
    }

    pub fn output(&self) -> Result<Tensor> {
        self.attr(RKNN_QUERY_OUTPUT_ATTR)
    }

    /// Bytes of activation memory one run needs.
    pub fn internal_size(&self) -> Result<u32> {
        // SAFETY: rknn_mem_size is plain data, valid when zeroed.
        let mut size: rknn_mem_size = unsafe { std::mem::zeroed() };
        self.query(RKNN_QUERY_MEM_SIZE, &mut size)?;
        Ok(size.total_internal_size)
    }

    /// API and driver versions.
    pub fn versions(&self) -> Result<(String, String)> {
        // SAFETY: rknn_sdk_version is plain data, valid when zeroed.
        let mut version: rknn_sdk_version = unsafe { std::mem::zeroed() };
        self.query(RKNN_QUERY_SDK_VERSION, &mut version)?;
        Ok((c_string(&version.api_version), c_string(&version.drv_version)))
    }

    pub fn set_core(&mut self, core: u32) -> Result<()> {
        // SAFETY: plain call on a live context.
        check(unsafe { (self.rt.set_core_mask)(self.ctx, core) }, "rknn_set_core_mask")
    }

    /// Runs on `mem`, which must stay alive while this context is used.
    pub fn set_internal_mem(&mut self, mem: &InternalMem) -> Result<()> {
        // SAFETY: `mem` was created by rknn_create_mem and is large enough
        // (InternalMem is sized from internal_size of every user).
        check(unsafe { (self.rt.set_internal_mem)(self.ctx, mem.mem) }, "rknn_set_internal_mem")
    }

    /// Runs one NHWC u8 image through the model.
    pub fn run(&mut self, input: &[u8]) -> Result<()> {
        let mut tensor = rknn_input {
            index: 0,
            buf: input.as_ptr().cast_mut().cast(),
            size: u32::try_from(input.len()).context("input too large")?,
            pass_through: 0,
            type_: RKNN_TENSOR_UINT8,
            fmt: RKNN_TENSOR_NHWC,
        };
        // SAFETY: `input` outlives both calls; librknnrt only reads it.
        check(unsafe { (self.rt.inputs_set)(self.ctx, 1, &mut tensor) }, "rknn_inputs_set")?;
        // SAFETY: plain call on a live context with its inputs set.
        check(unsafe { (self.rt.run)(self.ctx, std::ptr::null_mut()) }, "rknn_run")
    }

    fn output_into(&mut self, buf: *mut c_void, bytes: usize, want_float: bool) -> Result<()> {
        let mut tensor = rknn_output {
            want_float: u8::from(want_float),
            is_prealloc: 1,
            index: 0,
            buf,
            size: u32::try_from(bytes).context("output too large")?,
        };
        // SAFETY: `buf` is writable for `bytes` bytes; with is_prealloc the
        // library writes there and keeps nothing.
        check(
            unsafe { (self.rt.outputs_get)(self.ctx, 1, &mut tensor, std::ptr::null_mut()) },
            "rknn_outputs_get",
        )
    }

    /// The last run's output as f32.
    pub fn output_f32(&mut self, out: &mut [f32]) -> Result<()> {
        self.output_into(out.as_mut_ptr().cast(), std::mem::size_of_val(out), true)
    }

    /// The last run's FP16 output as raw half-precision bits, skipping the
    /// conversion to f32.
    pub fn output_f16(&mut self, out: &mut [u16]) -> Result<()> {
        self.output_into(out.as_mut_ptr().cast(), std::mem::size_of_val(out), false)
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        // SAFETY: the context is live and dropped once.
        unsafe { (self.rt.destroy)(self.ctx) };
    }
}

/// Activation memory shared by contexts that never run at the same time.
/// It is freed through the context that allocated it, so the owner of both
/// must drop it before that context.
pub struct InternalMem {
    rt: Arc<Runtime>,
    ctx: rknn_context,
    mem: *mut rknn_tensor_mem,
    size: u32,
}

// SAFETY: the buffer is only handed to librknnrt; see Context.
unsafe impl Send for InternalMem {}

impl InternalMem {
    /// Allocates `size` bytes through `ctx`, which must outlive the memory.
    pub fn new(ctx: &Context, size: u32) -> Result<Self> {
        // SAFETY: plain call on a live context.
        let mem = unsafe { (ctx.rt.create_mem)(ctx.ctx, size) };
        if mem.is_null() {
            return Err(anyhow!("rknn_create_mem of {size} bytes failed"));
        }
        Ok(Self { rt: ctx.rt.clone(), ctx: ctx.ctx, mem, size })
    }

    pub fn size(&self) -> u32 {
        self.size
    }
}

impl Drop for InternalMem {
    fn drop(&mut self) {
        // SAFETY: `mem` came from rknn_create_mem on `ctx`, which the owner
        // keeps alive until after this drop.
        unsafe { (self.rt.destroy_mem)(self.ctx, self.mem) };
    }
}
