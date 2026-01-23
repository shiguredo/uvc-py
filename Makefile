.PHONY: wheel develop test format clean

sync:
	uv sync

wheel: sync
	uv run maturin build --release

develop: sync
	uv run maturin develop

test: develop
	NO_UV_SYNC=1 uv run pytest --timeout=60

format:
	cargo fmt
	uv run ruff format examples/ python/uvc/ tests/

clean:
	rm -rf target dist *.egg-info
