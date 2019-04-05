package io.github.angelsl.javaeval;

import jdk.jshell.*;
import jnr.unixsocket.UnixServerSocketChannel;
import jnr.unixsocket.UnixSocketChannel;
import jnr.unixsocket.UnixSocketUtil;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.PrintStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.stream.Stream;

public class JavaEval {
    private static class Request {
        private final int timeout;
        private final String key;
        private final String code;
        private final UnixSocketChannel client;

        public Request(UnixSocketChannel client, int timeout, String key, String code) {
            this.client = client;
            this.timeout = timeout;
            this.key = key;
            this.code = code;
        }

        public int getTimeout() {
            return timeout;
        }

        public String getKey() {
            return key;
        }

        public String getCode() {
            return code;
        }

        public UnixSocketChannel getClient() {
            return client;
        }
    }

    private static class Response {
        private final UnixSocketChannel client;
        private final String response;

        public Response(UnixSocketChannel client, String response) {
            this.client = client;
            this.response = response;
        }

        public UnixSocketChannel getClient() {
            return client;
        }

        public String getResponse() {
            return response;
        }
    }

    private static class Context {
        private final JShell shell;
        private final ByteArrayOutputStream output;
        private final PrintStream printStream;
        private String buffer;

        public Context() {
            output = new ByteArrayOutputStream();
            printStream = new PrintStream(output);
            shell = JShell.builder().out(printStream).build();
            buffer = "";
        }

        public JShell getShell() {
            return shell;
        }

        public ByteArrayOutputStream getOutput() {
            return output;
        }

        public PrintStream getPrintStream() {
            return printStream;
        }

        public String getBuffer() {
            return buffer;
        }

        public void setBuffer(String buffer) {
            this.buffer = buffer == null ? "" : buffer;
        }
    }

    private static HashMap<String, Context> contexts = new HashMap<>();

    private static Context getContext(String key) {
        Context context = contexts.get(key);
        if (context == null) {
            context = new Context();
            contexts.put(key, context);
        }
        return context;
    }

    public static void main(String[] args) throws Throwable {
        UnixServerSocketChannel server = UnixSocketUtil.fromFD(3);
        Stream
            .generate(() -> {
                while (true) {
                    try {
                        return server.accept();
                    } catch (IOException e) {
                        System.err.println("exception accepting; stack trace follows");
                        e.printStackTrace();
                    }
                }
            })
            .parallel()
            .flatMap(JavaEval::readRequest)
            .map(request -> {
                Context c = getContext(request.getKey());
                JShell j = c.getShell();
                SourceCodeAnalysis sca = j.sourceCodeAnalysis();
                ByteArrayOutputStream baos = c.getOutput();
                PrintStream ps = c.getPrintStream();
                String code = c.getBuffer() + request.getCode();
                boolean needMore = false;
                outer: while (true) {
                    SourceCodeAnalysis.CompletionInfo ci = sca.analyzeCompletion(code);
                    switch (ci.completeness()) {
                        case DEFINITELY_INCOMPLETE:
                        case CONSIDERED_INCOMPLETE:
                            needMore = true;
                            break outer;
                        default:
                        case UNKNOWN:
                        case COMPLETE:
                        case COMPLETE_WITH_SEMI:
                            List<SnippetEvent> res = j.eval(code);
                            for (SnippetEvent se : res) {
                                if (se.previousStatus() == Snippet.Status.NONEXISTENT) {
                                    if (se.status() == Snippet.Status.VALID) {
                                        String val = se.value();
                                        JShellException ex = se.exception();
                                        if (ex != null) {
                                            if (ex instanceof EvalException) {
                                                EvalException ee = (EvalException) ex;
                                                ps.print(ee.getExceptionClassName());
                                                ps.println(" wrapped in");
                                            }
                                            ex.printStackTrace(ps);
                                        } else if (val != null) {
                                            ps.println(val);
                                        }
                                    } else {
                                        j.diagnostics(se.snippet()).forEach(d -> ps.println(d.getMessage(Locale.ENGLISH)));
                                    }
                                }
                            }
                            code = ci.remaining();
                            break;
                        case EMPTY:
                            break outer;
                    }
                }
                c.setBuffer(code);
                String output = needMore ? "(continue...)" : baos.toString(StandardCharsets.UTF_8);
                if (!needMore) {
                    baos.reset();
                }
                return new Response(request.getClient(), output);
            })
            .forEach(JavaEval::returnResponse);
    }

    private static boolean read(UnixSocketChannel client, ByteBuffer b) throws IOException {
        while (b.remaining() > 0) {
            if (client.read(b) == -1) {
                System.err.println("early eof while reading request");
                return true;
            }
        }
        b.flip();
        return false;
    }

    private static Stream<Request> readRequest(UnixSocketChannel client) {
        try {
            ByteBuffer b = ByteBuffer.allocateDirect(12).order(ByteOrder.LITTLE_ENDIAN);
            if (read(client, b)) return Stream.empty();
            final int timeout = b.getInt();
            final int keySize = b.getInt();
            final int codeSize = b.getInt();
            b = ByteBuffer.allocateDirect(keySize + codeSize).order(ByteOrder.LITTLE_ENDIAN);
            if (read(client, b)) return Stream.empty();
            final byte[] keyBytes = new byte[keySize];
            final byte[] codeBytes = new byte[codeSize];
            b.get(keyBytes);
            b.get(codeBytes);

            final String key = new String(keyBytes, StandardCharsets.UTF_8);
            final String code = new String(codeBytes, StandardCharsets.UTF_8);
            return Stream.of(new Request(client, timeout, key, code));
        } catch (IOException e) {
            System.err.println("exception reading request; stack trace follows");
            e.printStackTrace();
        }

        return Stream.empty();
    }

    private static void returnResponse(Response response) {
        try {
            final byte[] respBytes = response.getResponse().getBytes(StandardCharsets.UTF_8);
            ByteBuffer b = ByteBuffer.allocateDirect(4).order(ByteOrder.LITTLE_ENDIAN);
            b.putInt(respBytes.length);
            b.flip();
            response.getClient().write(b);
            response.getClient().write(ByteBuffer.wrap(respBytes));
        } catch (IOException e) {
            System.err.println("exception writing response; stack trace follows");
            e.printStackTrace();
        }
    }
}
