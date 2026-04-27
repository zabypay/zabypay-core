pub fn validate_strong_password(password: &str) -> Result<(), String> {
    let length_ok = password.len() >= 8;
    let has_upper = password.chars().any(|c| c.is_uppercase());
    let has_lower = password.chars().any(|c| c.is_lowercase());
    let has_digit = password.chars().any(|c| c.is_digit(10));
    let has_special = password.chars().any(|c| !c.is_alphanumeric());

    if !length_ok {
        return Err("Password must be at least 8 characters long.".to_string());
    }
    if !has_upper {
        return Err("Password must contain at least one uppercase letter.".to_string());
    }
    if !has_lower {
        return Err("Password must contain at least one lowercase letter.".to_string());
    }
    if !has_digit {
        return Err("Password must contain at least one digit.".to_string());
    }
    if !has_special {
        return Err("Password must contain at least one special character.".to_string());
    }

    Ok(())
}
