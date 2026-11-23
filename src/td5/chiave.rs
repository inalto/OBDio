//! Chiave dell'accesso di sicurezza del Td5, calcolata dal seme.
//!
//! Riscrittura in Rust di `keyGenerate` del file `keygen.c` del progetto
//! td5keygen (<https://github.com/pajacobson/td5keygen>), distribuito con
//! questa licenza, riportata integralmente anche in LICENSE-THIRD-PARTY.md:
//!
//! ```text
//! BSD 2-Clause License
//!
//! Copyright (c) 2017, paul@discotd5.com
//! All rights reserved.
//!
//! Redistribution and use in source and binary forms, with or without
//! modification, are permitted provided that the following conditions are met:
//!
//! * Redistributions of source code must retain the above copyright notice, this
//!   list of conditions and the following disclaimer.
//!
//! * Redistributions in binary form must reproduce the above copyright notice,
//!   this list of conditions and the following disclaimer in the documentation
//!   and/or other materials provided with the distribution.
//!
//! THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
//! AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
//! IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//! DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
//! FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
//! DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//! SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
//! CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
//! OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
//! OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
//! ```

/// Chiave a 16 bit per il seme a 16 bit (byte alto per primo nel messaggio).
pub fn chiave_da_seme(seme: u16) -> u16 {
    let mut seed = seme;
    let count =
        ((seed >> 0xC & 0x8) | (seed >> 0x5 & 0x4) | (seed >> 0x3 & 0x2) | (seed & 0x1)) + 1;
    for _ in 0..count {
        let tap = ((seed >> 1) ^ (seed >> 2) ^ (seed >> 8) ^ (seed >> 9)) & 1;
        let tmp = (seed >> 1) | (tap << 0xF);
        // La condizione guarda il seme del passo precedente, come in C.
        if (seed >> 0x3 & 1) == 1 && (seed >> 0xD & 1) == 1 {
            seed = tmp & !1;
        } else {
            seed = tmp | 1;
        }
    }
    seed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Valori calcolati a parte con una trascrizione in Python di keygen.c
    /// (non con questo codice). 34A5 → 54D3 è l'esempio del README di
    /// td5keygen.
    #[test]
    fn vettori_di_prova() {
        for (seme, chiave) in [
            (0x34A5, 0x54D3),
            (0x0000, 0x0001),
            (0xFFFF, 0x8081),
            (0x8091, 0x9AAC),
            (0x1234, 0x8247),
            (0xA5A5, 0x4533),
            (0x0001, 0x0001),
            (0x2008, 0x1004),
        ] {
            assert_eq!(chiave_da_seme(seme), chiave, "seme {seme:04X}");
        }
    }
}
