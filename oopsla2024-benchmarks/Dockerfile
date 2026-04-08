FROM rust:1.89-bookworm
RUN echo 'deb http://deb.debian.org/debian bookworm-backports main' >> /etc/apt/sources.list
RUN apt-get update && apt-get install -y git python3-matplotlib python3-pandas golang-1.21-go vim wget && \
    rm -rf /var/lib/apt/lists/*
ENV PATH="${PATH}:/usr/lib/go-1.21/bin"

## Install Maven 3.9+
RUN wget -qO /tmp/maven.tar.gz "https://archive.apache.org/dist/maven/maven-3/3.9.9/binaries/apache-maven-3.9.9-bin.tar.gz" && \
    mkdir -p /opt/maven && tar xzf /tmp/maven.tar.gz -C /opt/maven --strip-components=1 && rm /tmp/maven.tar.gz
ENV PATH="/opt/maven/bin:${PATH}"

## Install Eclipse Temurin JDK 25
RUN wget -qO /tmp/temurin.tar.gz "https://api.adoptium.net/v3/binary/latest/25/ga/linux/x64/jdk/hotspot/normal/eclipse" && \
    mkdir -p /opt/java && tar xzf /tmp/temurin.tar.gz -C /opt/java --strip-components=1 && rm /tmp/temurin.tar.gz
ENV JAVA_HOME="/opt/java"
ENV PATH="${JAVA_HOME}/bin:${PATH}"

## Get openfga
WORKDIR /
RUN git clone https://github.com/openfga/openfga -b main
# the openfga commit hash used in our experiments
RUN cd openfga && git checkout bbb4a07
COPY openfga-timing.patch openfga/
RUN cd openfga && git apply openfga-timing.patch
ENV GOPROXY="direct"
RUN cd openfga && go build -o ./openfga ./cmd/openfga
ENV PATH="${PATH}:/openfga"
## run an openfga server for 100ms and then kill it. For some reason this reduces flakiness later on
RUN timeout 0.1 openfga run --playground-enabled=false --log-level=warn || true

## Clone and build SAPL Policy Engine (install to local Maven repo)
WORKDIR /
RUN git clone https://github.com/heutelbeck/sapl-policy-engine.git -b master --depth 1
RUN cd sapl-policy-engine && mvn install -pl sapl-pdp -am -DskipTests -Dlicense.skip

## Get cedar-benchmarks (benchmark harness and example app code)
WORKDIR /
COPY Cargo.toml plot.py README.md cedar-benchmarks/
COPY generators cedar-benchmarks/generators/
COPY openfga-examples cedar-benchmarks/openfga-examples/
COPY src cedar-benchmarks/src/
COPY tinytodo cedar-benchmarks/tinytodo/
COPY validate_all_benchmarks.sh cedar-benchmarks

## Build Rego version of TinyTodo
RUN cd cedar-benchmarks/tinytodo/rego && \
    go mod init TinyTodoRego && \
    go get github.com/open-policy-agent/opa/rego && \
    mkdir -p build/bin && \
    go build -o build/bin/tinytodo cmd/tinytodo/main.go

## Build SAPL harness
COPY sapl-harness cedar-benchmarks/sapl-harness/
RUN cd cedar-benchmarks/sapl-harness && mvn package -DskipTests

## Build benchmark harness so it's ready
WORKDIR /cedar-benchmarks
RUN cargo clean && cargo build --release
