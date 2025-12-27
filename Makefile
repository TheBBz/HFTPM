.PHONY: help build test run clean install deploy-local setup deploy-hetzner docker-build docker-run check lint format security-scan

help:
	@echo "Available commands:"
	@echo "  make setup          - Run setup script"
	@echo "  make build          - Build release binary"
	@echo "  make test           - Run tests"
	@echo "  make run            - Run bot in dev mode"
	@echo "  make clean          - Clean build artifacts"
	@echo "  make install        - Install dependencies"
	@echo "  make deploy-local   - Deploy to local machine"
	@echo "  make deploy-hetzner - Deploy to Hetzner VPS"
	@echo "  make docker-build   - Build Docker image"
	@echo "  make docker-run     - Run bot in Docker"
	@echo "  make check          - Check dependencies and configuration"
	@echo "  make lint           - Run linter"
	@echo "  make format         - Format code"
	@echo "  make security-scan - Run security audit"

setup:
	@./setup.sh

build:
	@echo "Building release version..."
	cargo build --release --features jemalloc

test:
	@echo "Running tests..."
	cargo test --release

run:
	@echo "Running bot in dev mode..."
	cargo run

clean:
	@echo "Cleaning build artifacts..."
	rm -rf target/
	rm -rf logs/*.log

install:
	@echo "Installing dependencies..."
	cargo install --locked --path .

deploy-local:
	@echo "Deploying to local machine..."
	sudo cp config/hfptm.service /etc/systemd/system/
	sudo systemctl daemon-reload
	sudo systemctl enable hfptm
	sudo systemctl start hfptm
	@echo "Deployment complete. Check logs with: sudo journalctl -u hfptm -f"

deploy-hetzner:
	@echo "Deploying to Hetzner VPS..."
	@./deploy-hetzner.sh

docker-build:
	@echo "Building Docker image..."
	docker build -t hfptm:latest .

docker-run:
	@echo "Running bot in Docker..."
	docker run -d \
		--name hfptm-bot \
		-p 3000:3000 \
		-v $(pwd)/config:/app/config \
		-v $(pwd)/logs:/app/logs \
		hfptm:latest

check:
	@echo "Checking dependencies..."
	@command -v rustc &> /dev/null || (echo "❌ Rust not installed" && exit 1)
	@command -v cargo &> /dev/null || (echo "❌ Cargo not installed" && exit 1)
	@echo "✅ Rust $(rustc --version)"
	@echo "✅ Cargo $(cargo --version)"
	@echo ""
	@echo "Checking configuration..."
	@ls -la config/ || (echo "❌ Config directory not found" && exit 1)
	@test -f config/config.toml || (echo "⚠️  config.toml not found" && exit 1)
	@test -f config/secrets.toml || (echo "⚠️  secrets.toml not found" && exit 1)
	@echo "✅ Configuration files present"
	@echo ""
	@echo "Checking API connectivity..."
	@curl -s -o /dev/null -w "Polymarket API: %{http_code}\n" https://clob.polymarket.com/ok

lint:
	@echo "Running linter..."
	cargo clippy --all-targets -- -D warnings

format:
	@echo "Formatting code..."
	cargo fmt

security-scan:
	@echo "Running security scan..."
	cargo audit
	cargo deny check || true

logs:
	@echo "Showing logs..."
	tail -f logs/hfptm.log

monitoring-dashboard:
	@echo "Opening monitoring dashboard..."
	@echo "Navigate to: http://localhost:3000/metrics"
	@xdg-open http://localhost:3000/metrics || open http://localhost:3000/metrics || echo "Open http://localhost:3000/metrics in your browser"

stop:
	@echo "Stopping bot..."
	sudo systemctl stop hfptm || docker stop hfptm-bot

restart:
	@echo "Restarting bot..."
	sudo systemctl restart hfptm || docker restart hfptm-bot

status:
	@echo "Checking bot status..."
	sudo systemctl status hfptm || docker ps | grep hfptm
