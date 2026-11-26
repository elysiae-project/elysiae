FROM ubuntu:26.04

ARG CI_UID=1000
ARG CI_GID=1000

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install --yes --no-install-recommends \
    build-essential \
    ca-certificates \
    curl \
    desktop-file-utils \
    dbus-x11 \
    fontconfig \
    libfreetype-dev \
    libgtk-4-dev \
    libxml2-utils \
    meson \
    ninja-build \
    pkg-config \
    python3 \
    xauth \
    xvfb \
    && rm -rf /var/lib/apt/lists/*

RUN if ! getent group "$CI_GID" >/dev/null; then groupadd --gid "$CI_GID" ci; fi \
    && if ! getent passwd "$CI_UID" >/dev/null; then \
        useradd --uid "$CI_UID" --gid "$CI_GID" --create-home --shell /bin/bash ci; \
    fi

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal

ENV PATH="/root/.cargo/bin:${PATH}"

RUN pkg-config --atleast-version=4.22 gtk4 \
    && pkg-config --atleast-version=2.88 glib-2.0
