use bdk::bitcoin::{Address, Network, PrivateKey};
use bdk::bitcoin::bip32::DerivationPath;
use bdk::bitcoin::secp256k1::Secp256k1 as BdkSecp256k1;
use bdk::keys::{DerivableKey, ExtendedKey};
use bdk::keys::bip39::{Language, Mnemonic, WordCount};
use bdk::miniscript::Segwitv0;
use crate::shared::utils::errors::AppError;
// use bip39::{Language, Mnemonic, MnemonicType};

pub fn generate_bitcoin_wallet(
    mnemonic_str: &str,
    index: u32// Network::Bitcoin or Network::Testnet/Signet/Regtest
) -> Result<(String, String, String), AppError> {
  
    let mnemonic = Mnemonic::parse_in(Language::English, mnemonic_str)
        .map_err(|_| AppError::ValidationError("Invalid mnemonic phrase".to_string()))?;

    let xkey: ExtendedKey<Segwitv0> = mnemonic.into_extended_key()
        .map_err(|_| AppError::InternalServerError("Failed to create extended key".to_string()))?;

    let xprv = xkey
        .into_xprv(Network::Bitcoin)
        .ok_or_else(|| AppError::InternalServerError("Failed to create extended private key".to_string()))?;

    // 3) Use BIP84 (P2WPKH bech32). Coin type: 0' (mainnet) / 1' (testnet)
    let coin_type: u32 = if matches!(Network::Bitcoin, Network::Bitcoin) { 0 } else { 1 };
    let derivation_path_str = format!("m/84'/{}'/0'/0/{}", coin_type, index);
    let derivation_path: DerivationPath = derivation_path_str
        .parse()
        .map_err(|_| AppError::ValidationError("Invalid derivation path".to_string()))?;

    // 4) Derive child key, build WIF, pubkey, and bech32 address
    let secp = BdkSecp256k1::new();
    let child = xprv
        .derive_priv(&secp, &derivation_path)
        .map_err(|_| AppError::InternalServerError("Failed to derive private key".to_string()))?;

    let private_key = PrivateKey {
        inner: child.private_key,
        compressed: true,
        network: Network::Bitcoin,
    };
    let wif = private_key.to_wif(); // base58 (WIF)

    let public_key = private_key.public_key(&secp);
    let address = Address::p2wpkh(&public_key, Network::Bitcoin)
        .map_err(|_| AppError::InternalServerError("Failed to generate P2WPKH address".to_string()))?
        .to_string();
    println!("mnemonic: {} \n, address: {} \n, Private_key: {} \n", mnemonic_str, address, wif);
    Ok((address, wif, mnemonic_str.to_string()))
}
