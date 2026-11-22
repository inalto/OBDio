// btleplug Source Code File
//
// Copyright 2020 Nonpolynomial Labs LLC. All rights reserved.
//
// Licensed under the BSD 3-Clause license. See LICENSE file in the project root
// for full license information.
//
// Some portions of this file are taken and/or modified from Rumble
// (https://github.com/mwylde/rumble), using a dual MIT/Apache License under the
// following copyright:
//
// Copyright (c) 2014 The Rust Project Developers

use crate::{Error, Result, api::CharPropFlags};
// OBDio: serve solo ai test, da quando `to_uuid` non passa più da una stringa.
#[cfg(test)]
use std::str::FromStr;
use uuid::Uuid;
use windows::core::GUID;
use windows::{
    Devices::Bluetooth::GenericAttributeProfile::{
        GattCharacteristicProperties, GattClientCharacteristicConfigurationDescriptorValue,
        GattCommunicationStatus,
    },
    Storage::Streams::{DataReader, IBuffer},
};

pub fn to_error(status: GattCommunicationStatus) -> Result<()> {
    if status == GattCommunicationStatus::AccessDenied {
        Err(Error::PermissionDenied)
    } else if status == GattCommunicationStatus::Unreachable {
        Err(Error::NotConnected)
    } else if status == GattCommunicationStatus::Success {
        Ok(())
    } else if status == GattCommunicationStatus::ProtocolError {
        Err(Error::NotSupported("ProtocolError".to_string()))
    } else {
        Err(Error::Other("Communication Error:".to_string().into()))
    }
}

pub fn to_descriptor_value(
    properties: GattCharacteristicProperties,
) -> GattClientCharacteristicConfigurationDescriptorValue {
    let notify = GattCharacteristicProperties::Notify;
    let indicate = GattCharacteristicProperties::Indicate;
    if properties & indicate == indicate {
        GattClientCharacteristicConfigurationDescriptorValue::Indicate
    } else if properties & notify == notify {
        GattClientCharacteristicConfigurationDescriptorValue::Notify
    } else {
        GattClientCharacteristicConfigurationDescriptorValue::None
    }
}

// OBDio: conversione diretta dai campi del GUID, senza passare da una stringa
// e senza `unwrap()`: è chiamata dal gestore degli annunci (callback WinRT),
// dove un panic diventa un abort dell'intero processo (Rust >= 1.81).
pub fn to_uuid(uuid: &GUID) -> Uuid {
    Uuid::from_fields(uuid.data1, uuid.data2, uuid.data3, &uuid.data4)
}

// OBDio: tollerante. Chiamata dal gestore degli annunci (callback WinRT): un
// errore di WinRT nel leggere il buffer dà un vettore vuoto invece di un panic
// (che lì sarebbe un abort del processo).
pub fn to_vec(buffer: &IBuffer) -> Vec<u8> {
    match try_to_vec(buffer) {
        Ok(data) => data,
        Err(e) => {
            log::debug!("to_vec: buffer WinRT illeggibile, ignorato: {e:?}");
            Vec::new()
        }
    }
}

// OBDio: variante con errore esplicito di `to_vec`.
pub fn try_to_vec(buffer: &IBuffer) -> windows::core::Result<Vec<u8>> {
    let reader = DataReader::FromBuffer(buffer)?;
    let len = reader.UnconsumedBufferLength()? as usize;
    let mut data = vec![0u8; len];
    reader.ReadBytes(&mut data)?;
    Ok(data)
}

#[allow(dead_code)]
pub fn to_guid(uuid: &Uuid) -> GUID {
    let (data1, data2, data3, data4) = uuid.as_fields();
    GUID::from_values(data1, data2, data3, data4.to_owned())
}

pub fn to_char_props(props: &GattCharacteristicProperties) -> CharPropFlags {
    let mut flags = CharPropFlags::default();
    if *props & GattCharacteristicProperties::Broadcast != GattCharacteristicProperties::None {
        flags |= CharPropFlags::BROADCAST;
    }
    if *props & GattCharacteristicProperties::Read != GattCharacteristicProperties::None {
        flags |= CharPropFlags::READ;
    }
    if *props & GattCharacteristicProperties::WriteWithoutResponse
        != GattCharacteristicProperties::None
    {
        flags |= CharPropFlags::WRITE_WITHOUT_RESPONSE;
    }
    if *props & GattCharacteristicProperties::Write != GattCharacteristicProperties::None {
        flags |= CharPropFlags::WRITE;
    }
    if *props & GattCharacteristicProperties::Notify != GattCharacteristicProperties::None {
        flags |= CharPropFlags::NOTIFY;
    }
    if *props & GattCharacteristicProperties::Indicate != GattCharacteristicProperties::None {
        flags |= CharPropFlags::INDICATE;
    }
    if *props & GattCharacteristicProperties::AuthenticatedSignedWrites
        != GattCharacteristicProperties::None
    {
        flags |= CharPropFlags::AUTHENTICATED_SIGNED_WRITES;
    }
    if *props & GattCharacteristicProperties::ExtendedProperties
        != GattCharacteristicProperties::None
    {
        flags |= CharPropFlags::EXTENDED_PROPERTIES;
    }
    flags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_uuid_to_guid_conversion() {
        let uuid_str = "10B201FF-5B3B-45A1-9508-CF3EFCD7BBAF";
        let uuid = Uuid::from_str(uuid_str).unwrap();

        let guid_converted = to_guid(&uuid);

        let guid_expected = GUID::try_from(uuid_str).unwrap();
        assert_eq!(guid_converted, guid_expected);
    }

    #[test]
    fn check_guid_to_uuid_conversion() {
        let uuid_str = "10B201FF-5B3B-45A1-9508-CF3EFCD7BBAF";
        let guid = GUID::try_from(uuid_str).unwrap();

        let uuid_converted = to_uuid(&guid);

        let uuid_expected = Uuid::from_str(uuid_str).unwrap();
        assert_eq!(uuid_converted, uuid_expected);
    }

    // OBDio: la conversione diretta coincide con quella vecchia (via stringa)
    // anche sui casi limite, e il giro UUID -> GUID -> UUID è esatto.
    #[test]
    fn obdio_to_uuid_matches_string_conversion() {
        for uuid_str in [
            "00000000-0000-0000-0000-000000000000",
            "FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF",
            "0000FFF0-0000-1000-8000-00805F9B34FB",
            "E7810A71-73AE-499D-8C15-FAA9AEF0C3F2",
            "10B201FF-5B3B-45A1-9508-CF3EFCD7BBAF",
        ] {
            let guid = GUID::try_from(uuid_str).unwrap();
            let via_string = Uuid::from_str(&format!("{:?}", guid)).unwrap();
            assert_eq!(to_uuid(&guid), via_string, "{uuid_str}");
            let uuid = Uuid::from_str(uuid_str).unwrap();
            assert_eq!(to_uuid(&to_guid(&uuid)), uuid, "{uuid_str}");
        }
    }

    // OBDio: `to_vec` legge un vero IBuffer WinRT (nessun hardware richiesto).
    #[test]
    fn obdio_to_vec_reads_winrt_buffers() {
        use windows::Storage::Streams::DataWriter;
        let writer = DataWriter::new().unwrap();
        writer.WriteBytes(&[0x41, 0x54, 0x5A, 0x0D]).unwrap();
        let buffer = writer.DetachBuffer().unwrap();
        assert_eq!(to_vec(&buffer), vec![0x41, 0x54, 0x5A, 0x0D]);
        assert_eq!(try_to_vec(&buffer).unwrap(), vec![0x41, 0x54, 0x5A, 0x0D]);

        let empty = DataWriter::new().unwrap().DetachBuffer().unwrap();
        assert_eq!(to_vec(&empty), Vec::<u8>::new());
    }
}
