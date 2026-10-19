use windows::Win32::Foundation::{HMODULE, HWND, RECT};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BOX, D3D11_CPU_ACCESS_WRITE, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_SINGLETHREADED, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, DXGI_MWA_NO_ALT_ENTER, DXGI_SCALING_NONE,
    DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIFactory2,
    IDXGISwapChain1,
};

use super::Area;

pub(super) const GROWTH: u32 = 256;
pub(super) const BUFFERS: u32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Driver {
    Warp,
    Hardware,
}

impl Driver {
    fn d3d(self) -> D3D_DRIVER_TYPE {
        match self {
            Self::Warp => D3D_DRIVER_TYPE_WARP,
            Self::Hardware => D3D_DRIVER_TYPE_HARDWARE,
        }
    }

    fn other(self) -> Self {
        match self {
            Self::Warp => Self::Hardware,
            Self::Hardware => Self::Warp,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Warp => "warp",
            Self::Hardware => "hardware",
        }
    }

    fn probe(value: &str) -> Result<Self, String> {
        match value {
            "warp" => Ok(Self::Warp),
            "hardware" => Ok(Self::Hardware),
            _ => Err(format!("AKULA_EXPERIMENTAL_D3D11: expected hardware or warp, got {value:?}")),
        }
    }
}

// A diagnostic must never silently measure the fallback as the requested backend.
fn choose<T, E>(
    first: Driver,
    strict: bool,
    mut create: impl FnMut(Driver) -> Result<T, E>,
) -> Result<(T, Driver), E> {
    match create(first) {
        Ok(device) => Ok((device, first)),
        Err(error) if strict => Err(error),
        Err(_) => create(first.other()).map(|device| (device, first.other())),
    }
}

fn preference() -> Result<(Driver, bool), String> {
    // Experimental driver selection is unavailable in installed builds.
    // Ordinary builds preserve their pre-existing WARP-first policy.
    #[cfg(not(feature = "installed"))]
    if let Some(value) = std::env::var_os("AKULA_EXPERIMENTAL_D3D11") {
        let value = value.into_string().map_err(|_| "AKULA_EXPERIMENTAL_D3D11: invalid text".to_owned())?;
        return Driver::probe(&value).map(|driver| (driver, true));
    }
    #[cfg(feature = "memory-probe")]
    let driver = if std::env::var_os("AQ_PROBE_HARDWARE").is_some() { Driver::Hardware } else { Driver::Warp };
    #[cfg(not(feature = "memory-probe"))]
    let driver = Driver::Warp;
    Ok((driver, false))
}

pub(super) struct Gpu {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    pub chain: IDXGISwapChain1,
    pub canvas: ID3D11Texture2D,
    pub stale: [Vec<Area>; BUFFERS as usize],
    pub back: usize,
}

pub(super) fn capacity(need: u32, have: u32) -> u32 {
    if need <= have { have } else { need.next_multiple_of(GROWTH) }
}

fn device(driver: D3D_DRIVER_TYPE) -> windows::core::Result<(ID3D11Device, ID3D11DeviceContext)> {
    let (mut device, mut context) = (None, None);
    unsafe {
        D3D11CreateDevice(None, driver, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_SINGLETHREADED, None, D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut context))?;
    }
    Ok((device.expect("устройство D3D11"), context.expect("контекст D3D11")))
}

pub(super) fn texture(device: &ID3D11Device, width: u32, height: u32) -> windows::core::Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
        MiscFlags: 0,
    };
    let mut texture = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
    Ok(texture.expect("текстура холста"))
}

pub(super) fn rect(a: Area) -> RECT {
    RECT { left: a.x as i32, top: a.y as i32, right: (a.x + a.w) as i32, bottom: (a.y + a.h) as i32 }
}

pub(super) fn region(a: Area) -> D3D11_BOX {
    D3D11_BOX { left: a.x, top: a.y, front: 0, right: a.x + a.w, bottom: a.y + a.h, back: 1 }
}

pub(super) fn create(hwnd: HWND, width: u32, height: u32) -> Result<Gpu, String> {
    let (first, strict) = preference()?;
    let ((device, context), actual) = choose(first, strict, |driver| device(driver.d3d()))
        .map_err(|error| {
            if strict {
                format!("D3D11 probe {} failed (strict: no fallback): {error}", first.label())
            } else {
                format!("D3D11: {error}")
            }
        })?;
    #[cfg(not(feature = "installed"))]
    if strict {
        eprintln!("AKULA_D3D11_PROBE requested={} actual={} strict=1", first.label(), actual.label());
    }
    #[cfg(feature = "installed")]
    let _ = actual;
    let factory: IDXGIFactory2 = unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)) }.map_err(|e| format!("DXGI: {e}"))?;
    let desc = DXGI_SWAP_CHAIN_DESC1 {
        Width: width,
        Height: height,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        Stereo: false.into(),
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: BUFFERS,
        Scaling: DXGI_SCALING_NONE,
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
        AlphaMode: DXGI_ALPHA_MODE_IGNORE,
        Flags: 0,
    };
    let chain = unsafe { factory.CreateSwapChainForHwnd(&device, hwnd, &desc, None, None) }.map_err(|e| format!("цепочка кадров: {e}"))?;
    unsafe {
        let _ = factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER);
    }
    let canvas = texture(&device, width, height).map_err(|e| format!("холст: {e}"))?;
    Ok(Gpu { device, context, chain, canvas, stale: Default::default(), back: 0 })
}

#[cfg(test)]
mod tests {
    use super::{choose, Driver};

    #[test]
    fn driver_probe_only_accepts_explicit_backends() {
        assert_eq!(Driver::probe("hardware").unwrap(), Driver::Hardware);
        assert_eq!(Driver::probe("warp").unwrap(), Driver::Warp);
        assert!(Driver::probe("").is_err());
        assert!(Driver::probe("Hardware").is_err());
        assert!(Driver::probe("auto").is_err());
    }

    #[test]
    fn explicit_hardware_failure_never_becomes_warp_measurement() {
        let mut tried = Vec::new();
        let result: Result<((), Driver), &str> = choose(Driver::Hardware, true, |driver| {
            tried.push(driver);
            Err("hardware unavailable")
        });
        assert_eq!(result.unwrap_err(), "hardware unavailable");
        assert_eq!(tried, [Driver::Hardware]);
    }

    #[test]
    fn ordinary_warp_first_failure_still_uses_hardware() {
        let mut tried = Vec::new();
        let result = choose(Driver::Warp, false, |driver| {
            tried.push(driver);
            match driver {
                Driver::Warp => Err("WARP unavailable"),
                Driver::Hardware => Ok(1),
            }
        });
        assert_eq!(result.unwrap(), (1, Driver::Hardware));
        assert_eq!(tried, [Driver::Warp, Driver::Hardware]);
    }

    #[test]
    fn ordinary_warp_success_does_not_open_hardware() {
        let mut tried = Vec::new();
        let result = choose(Driver::Warp, false, |driver| {
            tried.push(driver);
            Ok::<_, ()>(3)
        });
        assert_eq!(result.unwrap(), (3, Driver::Warp));
        assert_eq!(tried, [Driver::Warp]);
    }
}
