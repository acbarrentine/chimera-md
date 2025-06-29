# SSL Setup Guide for Chimera-md

This guide covers setting up SSL/TLS certificates for Chimera-md using either Traefik or Nginx as a reverse proxy.

## Quick Start

1. **Fix the basic Docker setup first:**
   ```bash
   cd docker
   docker compose up -d
   ```

2. **Set up SSL with your domain:**
   ```bash
   ./setup-ssl.sh docs.yourdomain.com admin@yourdomain.com
   ```

## Option 1: Traefik (Recommended)

Traefik provides automatic SSL certificate management with Let's Encrypt.

### Benefits
- Automatic SSL certificate generation and renewal
- Built-in dashboard for monitoring
- Service discovery via Docker labels
- Rate limiting and security headers included

### Setup
```bash
# 1. Update domain and email in the setup script
./setup-ssl.sh docs.yourdomain.com admin@yourdomain.com

# 2. Create the Traefik network
docker network create traefik-network

# 3. Start the services
docker compose -f traefik-compose.yaml up -d

# 4. Check logs
docker compose -f traefik-compose.yaml logs -f
```

### Access Points
- **Your docs:** https://docs.yourdomain.com
- **Traefik dashboard:** https://traefik.yourdomain.com (disable in production)

### Security Notes
- Dashboard API is enabled for initial setup - disable in production
- Rate limiting is set to 100 requests/minute average, 200 burst
- Security headers are automatically applied

## Option 2: Nginx

Traditional Nginx reverse proxy with manual SSL certificate management.

### Benefits
- More control over configuration
- Familiar for sysadmins
- Mature and stable
- Custom caching and security rules

### Setup
```bash
# 1. Update domain in configuration
./setup-ssl.sh docs.yourdomain.com admin@yourdomain.com

# 2. Start services
docker compose -f nginx-compose.yaml up -d

# 3. Get SSL certificate
docker compose -f nginx-compose.yaml run --rm certbot certonly \
  --webroot --webroot-path /var/www/certbot \
  --email admin@yourdomain.com --agree-tos --no-eff-email \
  -d docs.yourdomain.com

# 4. Restart Nginx to load certificates
docker compose -f nginx-compose.yaml restart nginx
```

### Certificate Renewal
Certificates auto-renew via the certbot container. Check renewal:
```bash
docker compose -f nginx-compose.yaml run --rm certbot renew --dry-run
```

## Security Features

Both configurations include:

### HTTP Security Headers
- `X-Frame-Options: DENY`
- `X-Content-Type-Options: nosniff`
- `X-XSS-Protection: 1; mode=block`
- `Strict-Transport-Security` (HSTS)
- Content Security Policy

### Rate Limiting
- **Traefik:** 100 req/min average, 200 burst
- **Nginx:** 10 req/sec, 20 burst

### SSL/TLS Configuration
- TLS 1.2 and 1.3 only
- Strong cipher suites
- OCSP stapling (Nginx)
- Perfect Forward Secrecy

## Monitoring

### Traefik
- Dashboard: `https://traefik.yourdomain.com`
- Metrics endpoint available
- Built-in access logs

### Nginx
- Access logs: `docker compose -f nginx-compose.yaml logs nginx`
- Error logs included
- Standard log format for analysis tools

## Troubleshooting

### DNS Issues
```bash
# Check DNS resolution
nslookup docs.yourdomain.com
dig docs.yourdomain.com
```

### Certificate Issues
```bash
# Check certificate status (Traefik)
docker compose -f traefik-compose.yaml exec traefik cat /data/acme.json

# Check certificate status (Nginx)
docker compose -f nginx-compose.yaml exec nginx openssl x509 -in /etc/letsencrypt/live/docs.yourdomain.com/fullchain.pem -text -noout
```

### Connection Issues
```bash
# Test SSL connection
openssl s_client -connect docs.yourdomain.com:443 -servername docs.yourdomain.com

# Check if ports are open
nmap -p 80,443 your-server-ip
```

### Service Issues
```bash
# Check container status
docker compose -f traefik-compose.yaml ps
docker compose -f nginx-compose.yaml ps

# View logs
docker compose -f traefik-compose.yaml logs -f traefik
docker compose -f nginx-compose.yaml logs -f nginx
```

## Production Hardening

### Traefik
1. Disable dashboard API:
   ```yaml
   # Remove or change in traefik-compose.yaml
   - --api.insecure=false
   ```

2. Secure dashboard access:
   ```yaml
   labels:
     - "traefik.http.routers.traefik.middlewares=auth"
     - "traefik.http.middlewares.auth.basicauth.users=admin:$$2y$$10$$..."
   ```

### Nginx
1. Hide server version:
   ```nginx
   server_tokens off;
   ```

2. Add fail2ban for additional protection:
   ```bash
   # Install fail2ban on host system
   sudo apt install fail2ban
   ```

### General
1. **Firewall:** Allow only ports 80, 443, and SSH
2. **Updates:** Keep containers updated regularly
3. **Monitoring:** Set up log monitoring and alerting
4. **Backups:** Backup SSL certificates and configuration

## Performance Optimization

### Caching
Both configurations include appropriate caching headers for static files.

### Compression
Gzip compression is enabled for text-based content.

### HTTP/2
Both Traefik and Nginx support HTTP/2 for better performance.

### Resource Limits
Consider adding resource limits to containers:
```yaml
deploy:
  resources:
    limits:
      memory: 512M
      cpus: '0.5'
```

## Migration from HTTP

If you're migrating from HTTP-only:

1. **Test SSL setup** on a staging domain first
2. **Update all internal links** to use HTTPS
3. **Set up redirects** from HTTP to HTTPS (included in configs)
4. **Update any external integrations** to use HTTPS URLs
5. **Monitor certificate expiration** and renewal

## Support

For issues specific to:
- **Chimera-md:** Check the main project repository
- **Traefik:** https://doc.traefik.io/traefik/
- **Nginx:** https://nginx.org/en/docs/
- **Let's Encrypt:** https://letsencrypt.org/docs/