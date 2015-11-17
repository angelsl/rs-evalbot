#!/usr/bin/bash

if [ $USER != "root" ]
then
    echo "need root to do this"
    exit 1
fi

if [ -d sandbox ]
then
    echo "sandbox/ exists, bailing"
    exit 1
fi

set -o errexit -o nounset -o pipefail

cd "${BASH_SOURCE%/*}"

umask 022

mkdir sandbox

pacstrap -c -d sandbox \
    bash \
    coreutils \
    grep \
    dash \
    filesystem \
    glibc \
    pacman \
    procps-ng \
    shadow \
    util-linux \
    gcc \
    file gawk tar sed

mkdir sandbox/dev/shm
mknod -m 666 sandbox/dev/null c 1 3
mknod -m 644 sandbox/dev/urandom c 1 9
arch-chroot sandbox useradd -m rust

curl -L https://static.rust-lang.org/rustup.sh | \
    arch-chroot sandbox /usr/bin/sh -s - --prefix=/ --channel=nightly --yes --disable-sudo

cat <<EOF > sandbox/usr/local/bin/evaluate.sh
#!/usr/bin/dash

set -o errexit

rustc - -o ./out "\$@"
printf '\377' # 255 in octal
exec ./out
EOF
chmod a+x sandbox/usr/local/bin/evaluate.sh

arch-chroot sandbox pacman -Rcs --noconfirm file gawk tar sed
arch-chroot sandbox pacman -Scc --noconfirm

mksquashfs sandbox sandbox.sqfs
echo "Done; leaving sandbox/ here, you may want to delete it"

