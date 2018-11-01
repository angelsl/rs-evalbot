#!/usr/bin/bash

if [ $USER != "root" ]; then
    echo "need root to do this"
    exit 1
fi

set -o errexit -o nounset -o pipefail
umask 022

dir=$(mktemp -d)
mount -t tmpfs tmpfs $dir
pacstrap -c -d $dir \
    bash \
    filesystem \
    shadow \

mkdir $dir/dev/shm
mknod -m 666 $dir/dev/null c 1 3
mknod -m 644 $dir/dev/urandom c 1 9
arch-chroot $dir groupadd -g 717 eval
arch-chroot $dir useradd -m -u 717 -g 717 eval

rm -rf $dir/usr
rm -rf $dir/var
# mono has some configuration files (importantly, dllmaps)
# cp -R /etc/mono $dir/etc/mono

# you may want to consider copying any java files too, if needed
# cp -R /etc/java-jre8 $dir/etc/java-jre8

mkdir $dir/usr $dir/var
mkdir $dir/run/eval
chown 717:717 $dir/run/eval

mksquashfs $dir playpen.sqfs
umount $dir
rmdir $dir

echo <<EOF
made playpen squashfs image: playpen.sqfs

remember to mount /usr and /var into the playpen (readonly!):
# pushd playpen
# mount -o bind,ro /usr usr
# mount -o bind,ro /var var
EOF
