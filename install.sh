#!/bin/bash

# Ensure ~/.hexa/bin directory exists
mkdir -p ~/.hexa/bin/

# Copy hexa-nexus binary to ~/.hexa/bin/
cp target/release/hexa-nexus ~/.hexa/bin/hexa-nexus

# Restart hexa-nexus daemon
hexa nexus stop && hexa nexus start