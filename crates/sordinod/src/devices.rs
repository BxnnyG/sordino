//! Turning PipeWire registry objects into the device list the UI shows.

use std::collections::HashMap;

use pipewire::spa::pod::deserialize::PodDeserializer;
use pipewire::spa::pod::{Object, Value, ValueArray};
use pipewire::spa::sys;
use sordino_core::ipc::{Device, DeviceKind, ProfileInfo};
use sordino_core::profile::classify;

/// What we know about a physical audio source node.
#[derive(Clone, Debug)]
pub struct SourceNode {
    pub node_id: u32,
    pub name: String,
    pub description: String,
    pub card: Option<u32>,
    pub bus: Option<String>,
    pub form_factor: Option<String>,
    pub api: Option<String>,
    /// `card.profile.device`: which of the card's routes (ports) belongs to this node.
    pub profile_device: Option<i32>,
}

/// What we know about a device (card) object.
#[derive(Clone, Debug, Default)]
pub struct CardInfo {
    pub profiles: Vec<ProfileInfo>,
    pub active: Option<i32>,
    /// Active routes: where the card's volume and mute live (what `wpctl` and the desktop change).
    pub routes: Vec<RouteInfo>,
}

/// One active route (port) of a card with its volume and mute state.
#[derive(Clone, Debug, PartialEq)]
pub struct RouteInfo {
    pub index: i32,
    pub device: i32,
    /// `true` for outputs (playback), `false` for inputs (capture).
    pub output: bool,
    /// Linear channel volumes as PipeWire stores them.
    pub volumes: Vec<f32>,
    pub mute: bool,
}

impl RouteInfo {
    /// Volume on the user-facing (cubic) scale used by `wpctl`, KDE and GNOME.
    pub fn user_volume(&self) -> Option<f32> {
        if self.volumes.is_empty() {
            return None;
        }
        let avg = self.volumes.iter().sum::<f32>() / self.volumes.len() as f32;
        Some(avg.max(0.0).cbrt())
    }
}

impl SourceNode {
    pub fn from_props(node_id: u32, get: impl Fn(&str) -> Option<String>) -> Option<SourceNode> {
        Self::from_props_class(node_id, "Audio/Source", get)
    }

    /// Parse a node of the given `media.class` (`Audio/Source` or `Audio/Sink`).
    pub fn from_props_class(
        node_id: u32,
        class: &str,
        get: impl Fn(&str) -> Option<String>,
    ) -> Option<SourceNode> {
        if get("media.class")? != class {
            return None;
        }
        let name = get("node.name")?;
        Some(SourceNode {
            node_id,
            description: get("node.description")
                .or_else(|| get("node.nick"))
                .unwrap_or_else(|| name.clone()),
            name,
            card: get("device.id").and_then(|v| v.parse().ok()),
            bus: get("device.bus"),
            form_factor: get("device.form-factor"),
            api: get("device.api"),
            profile_device: get("card.profile.device").and_then(|v| v.parse().ok()),
        })
    }

    pub fn kind(&self) -> DeviceKind {
        let name = self.name.to_lowercase();
        let desc = self.description.to_lowercase();
        if name.contains("snd_aloop") || desc.contains("loopback") || desc.contains("schleifen") {
            return DeviceKind::Loopback;
        }
        if desc.contains("webcam") || desc.contains("camera") || desc.contains("kamera") {
            return DeviceKind::Webcam;
        }
        if self.api.as_deref() == Some("bluez5") || name.starts_with("bluez") {
            return DeviceKind::Bluetooth;
        }
        match self.form_factor.as_deref() {
            Some("headset") | Some("headphone") | Some("hands-free") => return DeviceKind::Headset,
            Some("webcam") => return DeviceKind::Webcam,
            Some("internal") => return DeviceKind::Builtin,
            _ => {}
        }
        match self.bus.as_deref() {
            Some("usb") => DeviceKind::Usb,
            Some("pci") => DeviceKind::Builtin,
            _ => DeviceKind::Other,
        }
    }

    /// Webcams, loopbacks and similar are hidden unless the user asks for everything.
    /// Kind of an output device, from the form factor and names.
    pub fn sink_kind(&self) -> DeviceKind {
        let name = self.name.to_lowercase();
        let desc = self.description.to_lowercase();
        if name.contains("snd_aloop") || desc.contains("loopback") {
            return DeviceKind::Loopback;
        }
        if self.api.as_deref() == Some("bluez5") || name.starts_with("bluez") {
            return DeviceKind::Bluetooth;
        }
        if name.contains("hdmi") || desc.contains("hdmi") || desc.contains("displayport") {
            return DeviceKind::Hdmi;
        }
        match self.form_factor.as_deref() {
            Some("headphone") => return DeviceKind::Headphones,
            Some("headset") | Some("hands-free") => return DeviceKind::Headset,
            Some("speaker") | Some("internal") => return DeviceKind::Speaker,
            _ => {}
        }
        match self.bus.as_deref() {
            Some("usb") => DeviceKind::Usb,
            Some("pci") => DeviceKind::Builtin,
            _ => DeviceKind::Other,
        }
    }

    pub fn hidden_by_default(&self) -> bool {
        matches!(self.kind(), DeviceKind::Webcam | DeviceKind::Loopback)
    }

    pub fn to_sink_device(&self, cards: &HashMap<u32, CardInfo>) -> Device {
        Device {
            kind: self.sink_kind(),
            ..self.to_device(cards)
        }
    }

    pub fn to_device(&self, cards: &HashMap<u32, CardInfo>) -> Device {
        let profile = self.card.and_then(|c| cards.get(&c)).and_then(|c| {
            let active = c.active?;
            c.profiles.iter().find(|p| p.index == active).cloned()
        });
        Device {
            id: self.name.clone(),
            node_id: self.node_id,
            name: self.description.clone(),
            kind: self.kind(),
            card: self.card,
            profile,
        }
    }
}

/// Parse one `EnumProfile` pod.
pub fn parse_enum_profile(bytes: &[u8]) -> Option<ProfileInfo> {
    let (_, value) = PodDeserializer::deserialize_any_from(bytes).ok()?;
    let Value::Object(Object { properties, .. }) = value else {
        return None;
    };
    let mut index = None;
    let mut name = None;
    let mut description = None;
    let mut priority = 0;
    let mut available = true;
    for p in properties {
        match (p.key, p.value) {
            (sys::SPA_PARAM_PROFILE_index, Value::Int(v)) => index = Some(v),
            (sys::SPA_PARAM_PROFILE_name, Value::String(v)) => name = Some(v),
            (sys::SPA_PARAM_PROFILE_description, Value::String(v)) => description = Some(v),
            (sys::SPA_PARAM_PROFILE_priority, Value::Int(v)) => priority = v,
            // SPA_PARAM_AVAILABILITY_no = 1
            (sys::SPA_PARAM_PROFILE_available, Value::Id(id)) => available = id.0 != 1,
            _ => {}
        }
    }
    let name = name?;
    Some(ProfileInfo {
        index: index?,
        kind: classify(&name),
        description: description.unwrap_or_else(|| name.clone()),
        name,
        available,
        priority,
    })
}

/// Parse the current `Profile` pod and return the active profile index.
pub fn parse_profile_index(bytes: &[u8]) -> Option<i32> {
    let (_, value) = PodDeserializer::deserialize_any_from(bytes).ok()?;
    let Value::Object(Object { properties, .. }) = value else {
        return None;
    };
    properties.into_iter().find_map(|p| match (p.key, p.value) {
        (sys::SPA_PARAM_PROFILE_index, Value::Int(v)) => Some(v),
        _ => None,
    })
}

/// Build the pod that selects a profile on a device.
pub fn profile_pod(index: i32) -> Option<Vec<u8>> {
    use pipewire::spa::pod::{serialize::PodSerializer, Property, PropertyFlags};
    let obj = Object {
        type_: sys::SPA_TYPE_OBJECT_ParamProfile,
        id: sys::SPA_PARAM_Profile,
        properties: vec![
            Property {
                key: sys::SPA_PARAM_PROFILE_index,
                flags: PropertyFlags::empty(),
                value: Value::Int(index),
            },
            // `save` makes WirePlumber remember the choice for this device.
            Property {
                key: sys::SPA_PARAM_PROFILE_save,
                flags: PropertyFlags::empty(),
                value: Value::Bool(true),
            },
        ],
    };
    PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &Value::Object(obj))
        .ok()
        .map(|(c, _)| c.into_inner())
}

/// Parse one `Route` pod.
pub fn parse_route(bytes: &[u8]) -> Option<RouteInfo> {
    let (_, value) = PodDeserializer::deserialize_any_from(bytes).ok()?;
    let Value::Object(Object { properties, .. }) = value else {
        return None;
    };
    let (mut index, mut device, mut output) = (None, None, false);
    let (mut volumes, mut mute) = (Vec::new(), false);
    for p in properties {
        match (p.key, p.value) {
            (sys::SPA_PARAM_ROUTE_index, Value::Int(v)) => index = Some(v),
            (sys::SPA_PARAM_ROUTE_device, Value::Int(v)) => device = Some(v),
            (sys::SPA_PARAM_ROUTE_direction, Value::Id(id)) => {
                output = id.0 == sys::SPA_DIRECTION_OUTPUT
            }
            (sys::SPA_PARAM_ROUTE_props, Value::Object(Object { properties, .. })) => {
                for q in properties {
                    match (q.key, q.value) {
                        (sys::SPA_PROP_channelVolumes, Value::ValueArray(ValueArray::Float(v))) => {
                            volumes = v
                        }
                        (sys::SPA_PROP_mute, Value::Bool(b)) => mute = b,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    Some(RouteInfo {
        index: index?,
        device: device?,
        output,
        volumes,
        mute,
    })
}

/// Build the pod that changes a route's volume (user scale, 0..1) and/or mute state. `save`
/// lets WirePlumber remember it, exactly like a change made in the desktop's sound settings.
pub fn route_pod(route: &RouteInfo, volume: Option<f32>, mute: Option<bool>) -> Option<Vec<u8>> {
    use pipewire::spa::pod::{serialize::PodSerializer, Property, PropertyFlags};
    let prop = |key, value| Property {
        key,
        flags: PropertyFlags::empty(),
        value,
    };
    let mut props = Vec::new();
    if let Some(v) = volume {
        let linear = v.clamp(0.0, 1.0).powi(3);
        let channels = route.volumes.len().max(1);
        props.push(prop(
            sys::SPA_PROP_channelVolumes,
            Value::ValueArray(ValueArray::Float(vec![linear; channels])),
        ));
    }
    if let Some(m) = mute {
        props.push(prop(sys::SPA_PROP_mute, Value::Bool(m)));
    }
    let direction = if route.output {
        sys::SPA_DIRECTION_OUTPUT
    } else {
        sys::SPA_DIRECTION_INPUT
    };
    let obj = Object {
        type_: sys::SPA_TYPE_OBJECT_ParamRoute,
        id: sys::SPA_PARAM_Route,
        properties: vec![
            prop(sys::SPA_PARAM_ROUTE_index, Value::Int(route.index)),
            prop(
                sys::SPA_PARAM_ROUTE_direction,
                Value::Id(pipewire::spa::utils::Id(direction)),
            ),
            prop(sys::SPA_PARAM_ROUTE_device, Value::Int(route.device)),
            prop(
                sys::SPA_PARAM_ROUTE_props,
                Value::Object(Object {
                    type_: sys::SPA_TYPE_OBJECT_Props,
                    id: sys::SPA_PARAM_Route,
                    properties: props,
                }),
            ),
            prop(sys::SPA_PARAM_ROUTE_save, Value::Bool(true)),
        ],
    };
    PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &Value::Object(obj))
        .ok()
        .map(|(c, _)| c.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, desc: &str, bus: Option<&str>, ff: Option<&str>) -> SourceNode {
        SourceNode {
            node_id: 1,
            name: name.into(),
            description: desc.into(),
            card: Some(2),
            bus: bus.map(Into::into),
            form_factor: ff.map(Into::into),
            api: None,
            profile_device: None,
        }
    }

    #[test]
    fn webcams_and_loopbacks_are_hidden() {
        assert!(node(
            "alsa_input.usb-Example_Studio_Webcam",
            "Studio Webcam Pro Analog Stereo",
            Some("usb"),
            Some("webcam")
        )
        .hidden_by_default());
        assert!(node(
            "alsa_input.platform-snd_aloop.0.analog-stereo",
            "Loopback Analog Stereo",
            None,
            None
        )
        .hidden_by_default());
        assert!(!node(
            "alsa_input.usb-Example_Mic_X1-00.mono-fallback",
            "Example Mic X1 Mono",
            Some("usb"),
            None
        )
        .hidden_by_default());
    }

    #[test]
    fn kinds() {
        assert_eq!(
            node("a", "USB Audio Device Mono", Some("usb"), None).kind(),
            DeviceKind::Usb
        );
        assert_eq!(
            node("a", "Internes Audio Analoges Stereo", Some("pci"), None).kind(),
            DeviceKind::Builtin
        );
        assert_eq!(
            node("bluez_input.x", "Headset", None, None).kind(),
            DeviceKind::Bluetooth
        );
    }

    #[test]
    fn only_sources_are_accepted() {
        let props = |k: &str| match k {
            "media.class" => Some("Audio/Sink".to_string()),
            "node.name" => Some("x".to_string()),
            _ => None,
        };
        assert!(SourceNode::from_props(1, props).is_none());
    }

    #[test]
    fn route_pod_roundtrips_on_the_user_scale() {
        let r = RouteInfo {
            index: 3,
            device: 7,
            output: false,
            volumes: vec![1.0],
            mute: false,
        };
        let back = parse_route(&route_pod(&r, Some(0.8), Some(true)).unwrap()).unwrap();
        assert_eq!((back.index, back.device, back.output), (3, 7, false));
        assert!(back.mute);
        assert!((back.user_volume().unwrap() - 0.8).abs() < 1e-4);
        assert!(
            (back.volumes[0] - 0.512).abs() < 1e-4,
            "stored linearly like wpctl"
        );
        let out = RouteInfo {
            output: true,
            volumes: vec![0.5, 0.5],
            ..r
        };
        let back = parse_route(&route_pod(&out, None, Some(false)).unwrap()).unwrap();
        assert!(back.output && !back.mute && back.volumes.is_empty());
    }

    #[test]
    fn profile_pod_roundtrips() {
        let bytes = profile_pod(5).unwrap();
        assert_eq!(parse_profile_index(&bytes), Some(5));
    }
}
