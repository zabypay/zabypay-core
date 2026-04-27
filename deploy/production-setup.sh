#!/bin/bash

# Production Deployment Script for Crypto Payment Gateway
# This script sets up the production environment with proper security and monitoring

set -e  # Exit on any error

echo " Setting up Crypto Payment Gateway for Production"
echo "=================================================="

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Check if running as root
if [ "$EUID" -eq 0 ]; then
    echo -e "${RED} Don't run this script as root for security reasons${NC}"
    exit 1
fi

# Function to check if command exists
command_exists() {
    command -v "$1" >/dev/null 2>&1
}

# Check prerequisites
echo -e "${YELLOW}📋 Checking prerequisites...${NC}"

if ! command_exists docker; then
    echo -e "${RED} Docker is not installed. Please install Docker first.${NC}"
    exit 1
fi

if ! command_exists docker-compose; then
    echo -e "${RED} Docker Compose is not installed. Please install Docker Compose first.${NC}"
    exit 1
fi

if ! command_exists rust; then
    echo -e "${RED} Rust is not installed. Please install Rust first.${NC}"
    exit 1
fi

echo -e "${GREEN}✅ All prerequisites met${NC}"

# Create production directory structure
echo -e "${YELLOW}📁 Creating directory structure...${NC}"
mkdir -p /opt/crypto-gateway/{logs,data,ssl,backups}
mkdir -p /opt/crypto-gateway/config

# Set up environment file
echo -e "${YELLOW}⚙️  Setting up environment configuration...${NC}"
if [ ! -f "/opt/crypto-gateway/.env" ]; then
    cp .env.production /opt/crypto-gateway/.env
    echo -e "${YELLOW}  Please edit /opt/crypto-gateway/.env with your production values${NC}"
    echo -e "${YELLOW}   Required changes:${NC}"
    echo -e "${YELLOW}   - DATABASE_URL${NC}"
    echo -e "${YELLOW}   - JWT_SECRET${NC}"
    echo -e "${YELLOW}   - ENCRYPTION_KEY${NC}"
    echo -e "${YELLOW}   - CORS_ALLOWED_ORIGINS${NC}"
    echo -e "${YELLOW}   - RPC endpoints and API keys${NC}"
    read -p "Press Enter when you've updated the .env file..."
else
    echo -e "${GREEN}✅ Environment file already exists${NC}"
fi

# Build the application
echo -e "${YELLOW}🔨 Building application...${NC}"
cargo build --release

# Copy binary to production location
echo -e "${YELLOW}📦 Installing application...${NC}"
cp target/release/api-crypto /opt/crypto-gateway/
cp -r migration /opt/crypto-gateway/

# Set up systemd service
echo -e "${YELLOW}🔧 Setting up systemd service...${NC}"
sudo tee /etc/systemd/system/crypto-gateway.service > /dev/null <<EOF
[Unit]
Description=Crypto Payment Gateway
After=network.target postgresql.service
Requires=postgresql.service

[Service]
Type=simple
User=$USER
WorkingDirectory=/opt/crypto-gateway
ExecStart=/opt/crypto-gateway/api-crypto
EnvironmentFile=/opt/crypto-gateway/.env
Restart=always
RestartSec=10
StandardOutput=append:/opt/crypto-gateway/logs/app.log
StandardError=append:/opt/crypto-gateway/logs/error.log

# Security settings
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/opt/crypto-gateway

[Install]
WantedBy=multi-user.target
EOF

# Set up log rotation
echo -e "${YELLOW}📋 Setting up log rotation...${NC}"
sudo tee /etc/logrotate.d/crypto-gateway > /dev/null <<EOF
/opt/crypto-gateway/logs/*.log {
    daily
    rotate 30
    compress
    delaycompress
    missingok
    notifempty
    create 644 $USER $USER
    postrotate
        systemctl reload crypto-gateway
    endscript
}
EOF

# Set up firewall rules
echo -e "${YELLOW}🔥 Configuring firewall...${NC}"
if command_exists ufw; then
    sudo ufw allow 8080/tcp comment 'Crypto Gateway API'
    sudo ufw allow 9090/tcp comment 'Crypto Gateway Admin'
    sudo ufw --force enable
    echo -e "${GREEN}✅ Firewall configured${NC}"
else
    echo -e "${YELLOW}  UFW not found. Please configure firewall manually:${NC}"
    echo -e "${YELLOW}   - Allow port 8080 for API${NC}"
    echo -e "${YELLOW}   - Allow port 9090 for Admin${NC}"
fi

# Set proper permissions
echo -e "${YELLOW}🔒 Setting permissions...${NC}"
chmod 755 /opt/crypto-gateway/api-crypto
chmod 600 /opt/crypto-gateway/.env
chown -R $USER:$USER /opt/crypto-gateway

# Run database migrations
echo -e "${YELLOW}🗄️  Running database migrations...${NC}"
cd /opt/crypto-gateway/migration
cargo run -- up

# Enable and start the service
echo -e "${YELLOW} Starting service...${NC}"
sudo systemctl daemon-reload
sudo systemctl enable crypto-gateway
sudo systemctl start crypto-gateway

# Wait for service to start
sleep 5

# Check service status
if sudo systemctl is-active --quiet crypto-gateway; then
    echo -e "${GREEN}✅ Service started successfully${NC}"
else
    echo -e "${RED} Service failed to start. Check logs:${NC}"
    echo -e "${RED}   sudo journalctl -u crypto-gateway -f${NC}"
    exit 1
fi

# Test health endpoint
echo -e "${YELLOW}🏥 Testing health endpoint...${NC}"
if curl -f http://localhost:8080/health > /dev/null 2>&1; then
    echo -e "${GREEN}✅ Health check passed${NC}"
else
    echo -e "${RED} Health check failed${NC}"
    echo -e "${YELLOW}   Check if the service is running: sudo systemctl status crypto-gateway${NC}"
fi

echo ""
echo -e "${GREEN}🎉 Production deployment completed successfully!${NC}"
echo ""
echo -e "${YELLOW} Service Information:${NC}"
echo -e "${YELLOW}   API Server: http://localhost:8080${NC}"
echo -e "${YELLOW}   Admin Panel: http://localhost:9090${NC}"
echo -e "${YELLOW}   Health Check: http://localhost:8080/health${NC}"
echo ""
echo -e "${YELLOW}🔧 Management Commands:${NC}"
echo -e "${YELLOW}   Start:   sudo systemctl start crypto-gateway${NC}"
echo -e "${YELLOW}   Stop:    sudo systemctl stop crypto-gateway${NC}"
echo -e "${YELLOW}   Status:  sudo systemctl status crypto-gateway${NC}"
echo -e "${YELLOW}   Logs:    sudo journalctl -u crypto-gateway -f${NC}"
echo ""
echo -e "${YELLOW}📁 Important Paths:${NC}"
echo -e "${YELLOW}   Application: /opt/crypto-gateway/${NC}"
echo -e "${YELLOW}   Logs: /opt/crypto-gateway/logs/${NC}"
echo -e "${YELLOW}   Config: /opt/crypto-gateway/.env${NC}"
echo ""
echo -e "${YELLOW}  Security Reminders:${NC}"
echo -e "${YELLOW}   1. Update JWT_SECRET and ENCRYPTION_KEY in production${NC}"
echo -e "${YELLOW}   2. Restrict CORS_ALLOWED_ORIGINS to your domains${NC}"
echo -e "${YELLOW}   3. Set up SSL/TLS certificates${NC}"
echo -e "${YELLOW}   4. Configure proper database backup strategy${NC}"
echo -e "${YELLOW}   5. Monitor logs regularly${NC}"
