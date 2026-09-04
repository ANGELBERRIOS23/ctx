#!/usr/bin/env bash
# ==============================================================================
# ctx-server Automated Deployment & Installation Script
#
# Generates secure random credentials for PostgreSQL, MinIO, and JWT auth,
# writes the .env configuration file, starts the services using Docker Compose,
# and displays connection endpoints and onboarding instructions.
# ==============================================================================

set -euo pipefail

# ANSI color codes
BOLD='\033[1m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

# Determine project root directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

echo -e "${BOLD}${BLUE}"
echo "  ██████╗████████╗██╗  ██╗"
echo " ██╔════╝╚══██╔══╝╚██╗██╔╝"
echo " ██║        ██║    ╚███╔╝ "
echo " ██║        ██║    ██╔██╗ "
echo " ╚██████╗   ██║   ██╔╝ ██╗"
echo "  ╚═════╝   ╚═╝   ╚═╝  ╚═╝"
echo " ctx-server Setup & Deployment"
echo -e "──────────────────────────────────────────${NC}"

# Check dependencies
command -v docker >/dev/null 2>&1 || {
    echo -e "${RED}Error: 'docker' is required but not installed or not in PATH.${NC}" >&2
    exit 1
}

if docker compose version >/dev/null 2>&1; then
    COMPOSE_CMD="docker compose"
elif command -v docker-compose >/dev/null 2>&1; then
    COMPOSE_CMD="docker-compose"
else
    echo -e "${RED}Error: Neither 'docker compose' nor 'docker-compose' was found.${NC}" >&2
    exit 1
fi

# Function to generate cryptographically random strings
generate_secret() {
    local length="${1:-32}"
    if command -v openssl >/dev/null 2>&1; then
        openssl rand -hex "$((length / 2))"
    elif [ -c /dev/urandom ]; then
        LC_ALL=C tr -dc 'A-Za-z0-9' < /dev/urandom | head -c "${length}"
    else
        echo -e "${RED}Error: Unable to generate random secrets. Neither openssl nor /dev/urandom available.${NC}" >&2
        exit 1
    fi
}

cd "${PROJECT_ROOT}"

# Handle existing .env file
ENV_FILE="${PROJECT_ROOT}/.env"
if [ -f "${ENV_FILE}" ]; then
    BACKUP_FILE="${PROJECT_ROOT}/.env.bak.$(date +%s)"
    echo -e "${YELLOW}Notice: Existing .env file found. Backing up to ${BACKUP_FILE}${NC}"
    cp "${ENV_FILE}" "${BACKUP_FILE}"
fi

echo -e "Generating cryptographically secure credentials..."

DB_USER="ctx"
DB_PASS="$(generate_secret 24)"
DB_NAME="ctx"
JWT_SECRET="$(generate_secret 48)"
MINIO_USER="ctxadmin"
MINIO_PASS="$(generate_secret 24)"
S3_BUCKET="ctx-blobs"
SERVER_PORT="${PORT:-9900}"

# Write .env configuration
cat <<EOF > "${ENV_FILE}"
# ctx-server environment configuration
# Generated on: $(date -u +"%Y-%m-%dT%H:%M:%SZ")

# Database (PostgreSQL)
POSTGRES_USER=${DB_USER}
POSTGRES_PASSWORD=${DB_PASS}
POSTGRES_DB=${DB_NAME}
DATABASE_URL=postgres://${DB_USER}:${DB_PASS}@db:5432/${DB_NAME}

# Object Storage (MinIO)
MINIO_ROOT_USER=${MINIO_USER}
MINIO_ROOT_PASSWORD=${MINIO_PASS}
BLOB_ENDPOINT=http://minio:9000
S3_ENDPOINT=http://minio:9000
S3_BUCKET=${S3_BUCKET}

# Authentication & Server
JWT_SECRET=${JWT_SECRET}
PORT=${SERVER_PORT}
RUST_LOG=info,ctx_server=debug
EOF

# Protect secret file permissions
chmod 600 "${ENV_FILE}"
echo -e "${GREEN}✓ Credentials and .env successfully written (chmod 600).${NC}"

# Launch Docker Compose stack
echo -e "Starting ctx services with '${COMPOSE_CMD}'..."
${COMPOSE_CMD} up -d --build

echo ""
echo -e "${BOLD}${GREEN}====================================================${NC}"
echo -e "${BOLD}${GREEN} ctx-server is running successfully!               ${NC}"
echo -e "${BOLD}${GREEN}====================================================${NC}"
echo ""
echo -e "${BOLD}Service Endpoints:${NC}"
echo -e "  • ctx-server API:    ${BLUE}http://localhost:${SERVER_PORT}${NC}"
echo -e "  • MinIO S3 API:      ${BLUE}http://localhost:9000${NC}"
echo -e "  • MinIO Web Console: ${BLUE}http://localhost:9001${NC}"
echo ""
echo -e "${BOLD}MinIO Console Credentials:${NC}"
echo -e "  • Username: ${BOLD}${MINIO_USER}${NC}"
echo -e "  • Password: ${BOLD}${MINIO_PASS}${NC}"
echo ""
echo -e "${BOLD}Database Connection:${NC}"
echo -e "  • URL: postgres://${DB_USER}:[HIDDEN]@localhost:5432/${DB_NAME}"
echo ""
echo -e "${BOLD}Next Steps:${NC}"
echo -e "  1. Install the CLI binary on your workstation:"
echo -e "     ${YELLOW}./scripts/install-cli.sh${NC}"
echo ""
echo -e "  2. Connect your CLI to this server:"
echo -e "     ${YELLOW}ctx connect http://localhost:${SERVER_PORT}${NC}"
echo ""
echo -e "  3. Authenticate or create your user account:"
echo -e "     ${YELLOW}ctx login${NC}"
echo ""
echo -e "  4. Check container status & logs:"
echo -e "     ${YELLOW}${COMPOSE_CMD} logs -f ctx-server${NC}"
echo ""
echo -e "  5. Stop services:"
echo -e "     ${YELLOW}${COMPOSE_CMD} down${NC}"
echo ""
