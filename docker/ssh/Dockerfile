# A small SSH server for the SSH tunnel tests: Alpine's OpenSSH, with the committed test keys
# (see generate.sh). sshd refuses a host key other users can read, and a copied file keeps the
# mode it had in the checkout.
FROM alpine:3.22
RUN apk add --no-cache openssh-server \
    && adduser -D dbrover \
    && echo 'dbrover:dbrover' | chpasswd
COPY sshd_config /etc/ssh/sshd_config
COPY ssh_host_ed25519_key ssh_host_ed25519_key.pub /etc/ssh/
COPY authorized_keys /etc/ssh/authorized_keys
RUN chmod 600 /etc/ssh/ssh_host_ed25519_key && chmod 644 /etc/ssh/authorized_keys
EXPOSE 22
CMD ["/usr/sbin/sshd", "-D", "-e"]
