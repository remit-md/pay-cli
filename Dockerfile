FROM gcr.io/distroless/cc-debian12
COPY pay-linux-amd64 /usr/local/bin/pay
ENTRYPOINT ["pay"]
