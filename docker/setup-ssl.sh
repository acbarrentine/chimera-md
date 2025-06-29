#!/bin/bash

# SSL Setup Script for Chimera-md
# This script helps set up SSL certificates for both Traefik and Nginx configurations

set -e

echo "🔐 SSL Setup for Chimera-md"
echo "============================="

# Check if domain is provided
if [ -z "$1" ]; then
    echo "Usage: $0 <your-domain.com> [email@example.com]"
    echo "Example: $0 docs.example.com admin@example.com"
    exit 1
fi

DOMAIN=$1
EMAIL=${2:-"admin@${DOMAIN}"}

echo "Domain: $DOMAIN"
echo "Email: $EMAIL"
echo ""

# Function to setup Traefik SSL
setup_traefik() {
    echo "🚀 Setting up Traefik SSL..."
    
    # Create network if it doesn't exist
    docker network create traefik-network 2>/dev/null || true
    
    # Update domain in traefik compose file
    sed -i "s/docs.yourdomain.com/$DOMAIN/g" traefik-compose.yaml
    sed -i "s/traefik.yourdomain.com/traefik.$DOMAIN/g" traefik-compose.yaml
    sed -i "s/your-email@example.com/$EMAIL/g" traefik-compose.yaml
    
    # Set proper permissions for ACME file
    mkdir -p traefik/data
    touch traefik/data/acme.json
    chmod 600 traefik/data/acme.json
    
    echo "✅ Traefik configuration updated"
    echo "📋 To start: docker compose -f traefik-compose.yaml up -d"
    echo "📋 Dashboard will be available at: https://traefik.$DOMAIN"
    echo "📋 Your site will be available at: https://$DOMAIN"
}

# Function to setup Nginx SSL
setup_nginx() {
    echo "🌐 Setting up Nginx SSL..."
    
    # Update domain in nginx config
    sed -i "s/docs.yourdomain.com/$DOMAIN/g" nginx/conf.d/chimera-md.conf
    
    # Create directories for certbot
    mkdir -p certbot/conf certbot/www
    
    echo "✅ Nginx configuration updated"
    echo "📋 To start:"
    echo "   1. docker compose -f nginx-compose.yaml up -d"
    echo "   2. docker compose -f nginx-compose.yaml run --rm certbot certonly --webroot --webroot-path /var/www/certbot --email $EMAIL --agree-tos --no-eff-email -d $DOMAIN"
    echo "   3. docker compose -f nginx-compose.yaml restart nginx"
    echo "📋 Your site will be available at: https://$DOMAIN"
}

# Ask user which setup they want
echo "Which reverse proxy would you like to set up?"
echo "1) Traefik (recommended - automatic SSL)"
echo "2) Nginx (manual SSL setup)"
echo "3) Both"
read -p "Enter your choice (1-3): " choice

case $choice in
    1)
        setup_traefik
        ;;
    2)
        setup_nginx
        ;;
    3)
        setup_traefik
        echo ""
        setup_nginx
        ;;
    *)
        echo "Invalid choice. Exiting."
        exit 1
        ;;
esac

echo ""
echo "🎉 SSL setup complete!"
echo ""
echo "⚠️  Important notes:"
echo "   - Make sure your domain DNS points to this server"
echo "   - Allow ports 80 and 443 through your firewall"
echo "   - For production, disable Traefik dashboard API access"
echo "   - Monitor SSL certificate renewal"
echo ""
echo "📖 Check the README for additional configuration options"