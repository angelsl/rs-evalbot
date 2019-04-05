package jnr.unixsocket;

import jnr.enxio.channels.NativeSelectorProvider;

import java.nio.channels.spi.SelectorProvider;

public class UnixSocketUtil {
    public static UnixServerSocketChannel fromFD(int fd) {
        return new UnixServerSocketChannel(NativeSelectorProvider.getInstance(), fd);
    }
}
