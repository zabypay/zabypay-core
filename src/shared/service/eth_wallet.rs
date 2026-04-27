use crate::shared::utils::errors::AppError;
use anyhow::{anyhow, Result};
use bip39::{Language, Mnemonic, MnemonicType};
use ethers_signers::{coins_bip39::English, MnemonicBuilder, Signer};
use hex;
/// Generate or reuse a mnemonic and derive an EVM wallet (ETH/BNB/etc.)
/// Returns: (address_checksum, private_key_hex, mnemonic_phrase)
pub fn generate_evm_wallet(mnemonic_opt: &str, index: u32) -> Result<(String, String, String)> {
    // 1) Use the provided mnemonic or generate a new one
    let mnemonic_phrase = if mnemonic_opt.is_empty() {
        Mnemonic::new(MnemonicType::Words24, Language::English)
            .phrase()
            .to_string()
    } else {
        mnemonic_opt.to_string()
    };

    // 2) Derive wallet at m/44'/60'/0'/0/index  (60' = EVM/ETH/BNB coin type)
    let derivation_path = format!("m/44'/60'/0'/0/{index}");
    let wallet = MnemonicBuilder::<English>::default()
        .phrase(mnemonic_phrase.as_str())
        .derivation_path(derivation_path.as_str())
        .map_err(|e| anyhow!("Bad derivation path: {e}"))?
        .build()
        .map_err(|e| anyhow!("Failed to build wallet: {e}"))?;

    // 3) Address (EIP‑55 checksum) and private key (hex)
    let address_checksum = format!("{:?}", wallet.address()); // Display impl yields checksum string
    let privkey_hex = hex::encode(wallet.signer().to_bytes());
    println!(
        "privkey_hex: {}  \n, address_checksum: {}  \n mnemonic_phrase: {}  \n",
        privkey_hex, address_checksum, mnemonic_phrase
    );

    Ok((address_checksum, privkey_hex, mnemonic_phrase))
}
