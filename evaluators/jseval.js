#!/usr/bin/env node

var cluster = require('cluster');
var util = require('util');
var vm = require('vm');

if (cluster.isMaster) {
    var data = new Buffer(0);

    cluster.setupMaster({
        exec: __filename,
        silent: true,
        args: []
    });
    var worker = cluster.fork();
    var outbuf = new Buffer(4);

    worker.on('message', function(message) {
        outbuf.writeInt32LE(Buffer.byteLength(message, 'utf8'), 0);
        process.stdout.write(outbuf);
        process.stdout.write(message, 'utf8');
    });

    process.stdin.on('data', function(chunk) {
        data = Buffer.concat([data, chunk]);
        var key_len = 0;
        var code_len = 0;
        if (data.length >= 12
            && data.length >= 12 + (key_len = data.readInt32LE(4)) + (code_len = data.readInt32LE(8))) {
            worker.send({
                timeout: data.readInt32LE(0),
                key: data.toString('utf8', 12, 12 + key_len),
                code: data.toString('utf8', 12 + key_len, 12 + key_len + code_len)
            });
            data = new Buffer(0);
        }
    });
} else {
    var getcontext = (function(newctx) {
        var contexts = new Map();
        return (function(key) {
            if (!contexts.has(key)) {
                var ctx = newctx();
                contexts.set(key, ctx);
                return ctx;
            } else {
                return contexts.get(key);
            }
        });
    })(function() {
        return {
            context: vm.createContext({
                console: console,
                module: module,
                process: process,
                require: require
            }),
            buf: ""
        };
    });
    var stdout;
    var callback = function(data) {
        stdout += data;
    };
    process.stdout.write = (function(write) {
        return function(string, encoding, fd) {
            callback.call(callback, string);
        };
    }(process.stdout.write));
    process.stderr.write = (function(write) {
        return function(string, encoding, fd) {
            callback.call(callback, string);
        };
    }(process.stderr.write));

    process.on('message', function(message) {
        var finished = true;
        var ctx = getcontext(message.key);
        ctx.buf += message.code;
        stdout = "";
        try {
            var out = vm.runInContext(ctx.buf, ctx.context, {
                filename: 'stdin',
                timeout: message.timeout
            });
            if (typeof out !== "undefined") {
                stdout += util.inspect(out);
            }
        } catch(err) {
            // FIXME hack hack hack
            if (err.name === "SyntaxError" && err.message === "Unexpected end of input") {
                finished = false;
            } else {
                stdout += err.toString();
            }
        }
        if (finished) {
            ctx.buf = "";
        }
        process.send(finished ? stdout : "(continue...)");
    });
}
