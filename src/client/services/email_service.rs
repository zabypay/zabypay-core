use crate::shared::utils::errors::AppError;
use lettre::{message::header::ContentType, Message, SmtpTransport, Transport};
use std::env;

pub struct EmailService;

impl EmailService {
    pub fn send_verification_email(email: &str, code: &str) -> Result<(), AppError> {
        let smtp_user = env::var("SMTP_USER").map_err(|_| AppError::EmailSendError)?;
        let smtp_pass = env::var("SMTP_PASS").map_err(|_| AppError::EmailSendError)?;
        let smtp_host = env::var("SMTP_HOST").map_err(|_| AppError::EmailSendError)?;

        let html_body = format!(
            r#"
            <div style='font-family: Arial, sans-serif; max-width: 600px; margin: 0 auto; padding: 20px; background: linear-gradient(135deg, #667eea 0%, #764ba2 100%); color: white; border-radius: 10px;'>
                <div style='text-align: center; margin-bottom: 30px;'>
                    <h1 style='color: #ffffff; font-size: 28px; margin: 0;'>🔐 Crypto Payment Gateway</h1>
                    <p style='color: #e0e6ed; margin: 5px 0 0 0;'>Secure Email Verification</p>
                </div>
                
                <div style='background: rgba(255,255,255,0.1); padding: 25px; border-radius: 8px; margin: 20px 0;'>
                    <h2 style='color: #ffffff; margin-top: 0;'>Welcome!</h2>
                    <p style='color: #e0e6ed; line-height: 1.6;'>
                        Thank you for registering with our crypto payment gateway. 
                        To complete your registration, please use the verification code below:
                    </p>
                    
                    <div style='text-align: center; margin: 25px 0;'>
                        <div style='background: rgba(255,255,255,0.2); padding: 15px 25px; border-radius: 6px; display: inline-block; border: 2px solid rgba(255,255,255,0.3);'>
                            <span style='font-size: 32px; font-weight: bold; color: #ffffff; letter-spacing: 3px; font-family: monospace;'>{}</span>
                        </div>
                    </div>
                    
                    <div style='background: rgba(255,255,255,0.1); padding: 15px; border-radius: 5px; border-left: 4px solid #ffd700;'>
                        <p style='color: #ffd700; margin: 0; font-weight: bold;'> Important:</p>
                        <p style='color: #e0e6ed; margin: 5px 0 0 0; font-size: 14px;'>
                            This verification code expires in <strong>10 minutes</strong>. 
                            Keep this code secure and do not share it with anyone.
                        </p>
                    </div>
                </div>
                
                <div style='text-align: center; margin-top: 30px; padding-top: 20px; border-top: 1px solid rgba(255,255,255,0.2);'>
                    <p style='color: #b8c6d0; font-size: 12px; margin: 0;'>
                        If you didn't request this verification code, please ignore this email.
                    </p>
                    <p style='color: #b8c6d0; font-size: 12px; margin: 5px 0 0 0;'>
                        © 2024 Crypto Payment Gateway - Secure by Design
                    </p>
                </div>
            </div>"#,
            code
        );

        let email_message = Message::builder()
            .from(
                format!("Crypto Payment Gateway <{}>", smtp_user)
                    .parse()
                    .unwrap(),
            )
            .to(email.parse().unwrap())
            .subject("🔐 Email Verification - Crypto Payment Gateway")
            .header(ContentType::TEXT_HTML)
            .body(html_body)
            .map_err(|_| AppError::EmailSendError)?;

        let creds = lettre::transport::smtp::authentication::Credentials::new(smtp_user, smtp_pass);
        let mailer = SmtpTransport::relay(&smtp_host)
            .map_err(|_| AppError::EmailSendError)?
            .credentials(creds)
            .build();

        mailer
            .send(&email_message)
            .map_err(|_| AppError::EmailSendError)?;

        Ok(())
    }

    pub fn send_welcome_email(email: &str, name: &str) -> Result<(), AppError> {
        let smtp_user = env::var("SMTP_USER").map_err(|_| AppError::EmailSendError)?;
        let smtp_pass = env::var("SMTP_PASS").map_err(|_| AppError::EmailSendError)?;
        let smtp_host = env::var("SMTP_HOST").map_err(|_| AppError::EmailSendError)?;

        let html_body = format!(
            r#"
            <div style='font-family: Arial, sans-serif; max-width: 600px; margin: 0 auto; padding: 20px; background: linear-gradient(135deg, #667eea 0%, #764ba2 100%); color: white; border-radius: 10px;'>
                <div style='text-align: center; margin-bottom: 30px;'>
                    <h1 style='color: #ffffff; font-size: 28px; margin: 0;'>🎉 Welcome to Crypto Payment Gateway</h1>
                </div>
                
                <div style='background: rgba(255,255,255,0.1); padding: 25px; border-radius: 8px; margin: 20px 0;'>
                    <h2 style='color: #ffffff; margin-top: 0;'>Hello {}!</h2>
                    <p style='color: #e0e6ed; line-height: 1.6;'>
                        Congratulations! Your email has been successfully verified and your account is now active.
                    </p>
                    
                    <div style='background: rgba(255,255,255,0.1); padding: 15px; border-radius: 5px; border-left: 4px solid #00ff88;'>
                        <p style='color: #00ff88; margin: 0; font-weight: bold;'>✅ What's Next:</p>
                        <ul style='color: #e0e6ed; margin: 10px 0 0 0; padding-left: 20px;'>
                            <li>Generate your first cryptocurrency wallet</li>
                            <li>Explore supported currencies (ETH, BTC, SOL, USDT, BDN)</li>
                            <li>Set up your merchant account for payment processing</li>
                        </ul>
                    </div>
                </div>
                
                <div style='text-align: center; margin-top: 30px;'>
                    <a href='http://localhost:8080/client/api/v1/dashboard' style='background: #ffd700; color: #333; padding: 12px 25px; text-decoration: none; border-radius: 5px; font-weight: bold; display: inline-block;'>
                        Access Your Dashboard
                    </a>
                </div>
                
                <div style='text-align: center; margin-top: 30px; padding-top: 20px; border-top: 1px solid rgba(255,255,255,0.2);'>
                    <p style='color: #b8c6d0; font-size: 12px; margin: 5px 0 0 0;'>
                        © 2024 Crypto Payment Gateway - Secure by Design
                    </p>
                </div>
            </div>"#,
            name
        );

        let email_message = Message::builder()
            .from(
                format!("Crypto Payment Gateway <{}>", smtp_user)
                    .parse()
                    .unwrap(),
            )
            .to(email.parse().unwrap())
            .subject("🎉 Welcome! Your account is now verified")
            .header(ContentType::TEXT_HTML)
            .body(html_body)
            .map_err(|_| AppError::EmailSendError)?;

        let creds = lettre::transport::smtp::authentication::Credentials::new(smtp_user, smtp_pass);
        let mailer = SmtpTransport::relay(&smtp_host)
            .map_err(|_| AppError::EmailSendError)?
            .credentials(creds)
            .build();

        mailer
            .send(&email_message)
            .map_err(|_| AppError::EmailSendError)?;

        Ok(())
    }
}
