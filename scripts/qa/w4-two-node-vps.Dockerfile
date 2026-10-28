# Ephemeral isolated SSH receiver for Akula's opt-in W3/W4 acceptance.
# CI-only, never deployed or published. Private keys are generated per run.
FROM ubuntu:22.04

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update -qq \
    && apt-get install -y -qq --no-install-recommends \
       openssh-server openssh-sftp-server \
       ca-certificates coreutils bash \
       libfuse2 libglib2.0-0 libx11-6 libxkbcommon0 \
       libxkbcommon-x11-0 libfontconfig1 libgtk-3-0 \
       libgstreamer1.0-0 libgstreamer-plugins-base1.0-0 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --shell /bin/bash w4qa \
    && usermod --password '*' w4qa \
    && install -d -m 0700 -o w4qa -g w4qa /home/w4qa/.ssh \
    && install -d -m 0755 /run/sshd \
    && printf '%s\n' \
       'PasswordAuthentication no' \
       'KbdInteractiveAuthentication no' \
       'PermitRootLogin no' \
       'PubkeyAuthentication yes' \
       'AuthenticationMethods publickey' \
       'AllowUsers w4qa' \
       'AllowAgentForwarding no' \
       'AllowTcpForwarding local' \
       'GatewayPorts no' \
       'X11Forwarding no' \
       'PermitUserEnvironment no' \
       'PermitTunnel no' \
       'LogLevel ERROR' \
       > /etc/ssh/sshd_config.d/akula-w4-fixture.conf \
    && printf 'ffffffffffffffffffffffffffffffff\n' > /etc/machine-id

EXPOSE 22
# The test driver mounts two ephemeral public/private test-key inputs and
# launches sshd itself, after installing them with restrictive permissions.
CMD ["/usr/sbin/sshd", "-D", "-e"]
