#!/bin/bash
set -eo pipefail

VAULT="/var/db/sudo-secretspec"
ENV_FILE="$VAULT/.env"
DB_FILE="$VAULT/secrets.db"

if [ "$1" == "--rollback" ]; then
    echo "Rolling back migration..."
    if [ -f "$DB_FILE.bak" ]; then
        mv "$DB_FILE.bak" "$DB_FILE"
        chown _sudo_secretspec:_sudo_secretspec "$DB_FILE"
        echo "Rollback complete!"
    else
        echo "No backup found to roll back to!"
    fi
    exit 0
fi

if [ ! -f "$ENV_FILE" ]; then
    echo "Error: $ENV_FILE does not exist. Are you running with sudo?"
    exit 1
fi

echo "Starting migration from .env to SQLite secrets.db..."

if [ -f "$DB_FILE" ]; then
    echo "Backing up existing secrets.db to secrets.db.bak..."
    cp -p "$DB_FILE" "$DB_FILE.bak"
fi

count=0
while IFS='=' read -r key value || [ -n "$key" ]; do
    if [[ -z "$key" || "$key" == \#* ]]; then
        continue
    fi
    
    # Strip quotes if present
    value="${value%\"}"
    value="${value#\"}"
    value="${value%\'}"
    value="${value#\'}"
    
    echo "Migrating $key..."
    # The CLI takes the value from stdin
    if ! echo -n "$value" | sudo-secretspec set "$key" --reason "Migrate from dotenv"; then
        echo "Secret $key not found in declarations. Adding it..."
        sudo-secretspec add "$key" --description "Migrated from dotenv" --reason "Migrate from dotenv"
        echo -n "$value" | sudo-secretspec set "$key" --reason "Migrate from dotenv"
    fi
    count=$((count+1))
done < "$ENV_FILE"

echo "Migration successful ($count secrets migrated). Run this script with --rollback to undo."
