# gpsd and gpsfake for the gpsd module's container test. gpsfake replays a
# recording through a pty into a private gpsd, which listens on port 2947.
# gpsfake starts the recording again when it reaches the end, so one
# container runs for as long as a test needs.
FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends gpsd gpsd-clients python3 \
    && rm -rf /var/lib/apt/lists/*
COPY neo-m8n.log /neo-m8n.log
EXPOSE 2947
# -n: gpsd reads the device at once, not only while a client watches.
# -o=-G: gpsd listens on all addresses; the port is published from the
# container, so loopback is not enough.
# -c 0.1: 0.1 s between sentences, about one fix per second.
CMD ["gpsfake", "-n", "-P", "2947", "-o=-G", "-c", "0.1", "/neo-m8n.log"]
