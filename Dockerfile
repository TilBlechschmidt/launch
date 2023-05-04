FROM caddy:2.6.4-alpine

RUN apk add --no-cache curl
RUN curl -LO https://dl.k8s.io/release/v1.26.3/bin/linux/amd64/kubectl \
    && chmod +x kubectl \
    && mv kubectl /usr/bin

RUN printf "set -e\n/launch server &\n caddy run\necho 'killing $!'\nkill $!\n" > /entrypoint.sh
RUN chmod +x /entrypoint.sh

COPY target/x86_64-unknown-linux-musl/release/launch /launch

EXPOSE 8080

ENTRYPOINT /entrypoint.sh
