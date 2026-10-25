# Test data for the gpsd module

`Dockerfile` builds the image for the container test in the `tests` module of
`../mod.rs`: Debian's gpsd and gpsd-clients, with gpsfake
replaying `neo-m8n.log` through a pty into gpsd on port 2947. The test
builds the image and starts the container itself; it needs a Docker daemon
(Docker Desktop, OrbStack, or Podman with a Docker-compatible socket).

To run the container by hand:

```
docker build -t gpsd-test src/gpsd/testdata
docker run --rm -p 127.0.0.1:2948:2947 gpsd-test
gpspipe -w 127.0.0.1:2948     # or: nc 127.0.0.1 2948, then ?WATCH={"enable":true,"json":true};
```

`neo-m8n.log` is `test/daemon/neo-m8n.log` from the gpsd repository
(https://gitlab.com/gpsd/gpsd), recorded by Gary E. Miller on 2015-03-18
from a BN-880 module with a u-blox NEO-M8N: NMEA with GPS and GLONASS
satellites and a 3D fix. gpsd is distributed under the BSD-2-Clause licence,
which this copy keeps. Its leading `#` lines are the recording's own notes.
