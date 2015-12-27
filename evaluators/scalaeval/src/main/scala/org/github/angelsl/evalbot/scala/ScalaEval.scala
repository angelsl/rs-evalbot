package org.github.angelsl.evalbot.scala

import java.io.{ByteArrayInputStream, ByteArrayOutputStream, DataInputStream, DataOutputStream, InputStream, OutputStream, PrintStream, PrintWriter}
import java.nio.charset.StandardCharsets

import scala.tools.nsc.Settings
import scala.tools.nsc.interpreter.{IMain, Results}

object ScalaEval {
  private val stdOut = System.out
  private val stdIn = System.in

  private val inRdr = new LittleEndianDataInputStream(stdIn)
  private val outWtr = new LittleEndianDataOutputStream(stdOut)

  private val evalOutBuf = new ByteArrayOutputStream()
  private val evalOut = new PrintStream(evalOutBuf)
  private val evalOutWtr = new PrintWriter(evalOut)
  private val evalIn = new ByteArrayInputStream(Array[Byte]())

  def main(args: Array[String]): Unit = {
    Console.setOut(evalOut)
    System.setOut(evalOut)

    Console.setErr(evalOut)
    System.setErr(evalOut)

    Console.setIn(evalIn)
    System.setIn(evalIn)

    mainLoop()
  }

  private def mainLoop(): Unit = {
    val intp = new IMain(new Settings(), evalOutWtr)
    var codeBuf = ""
    while (true) {
      val req = readReq()
      codeBuf += req.code
      intp.interpret(codeBuf) match {
        case Results.Incomplete => respond(success = false, "(continue...)")
        case _ =>
          codeBuf = ""
          respond(success = true, evalOutBuf.toByteArray)
          evalOutBuf.reset()
      }
    }
  }

  private def readReq(): Req = {
    val timeout = inRdr.readIntLE()
    val codeSz = inRdr.readIntLE()

    val codeBytes = new Array[Byte](codeSz)
    inRdr.readFully(codeBytes)

    new Req(new String(codeBytes, StandardCharsets.UTF_8), timeout)
  }

  private def respond(success: Boolean, resp: String): Unit =
    respond(success, resp.getBytes(StandardCharsets.UTF_8))

  private def respond(success: Boolean, resp: Array[Byte]): Unit = {
    outWtr.writeByte(if (success) 1 else 0)
    outWtr.writeIntLE(resp.length)
    outWtr.write(resp)
    outWtr.flush()
  }

  private class Req(codec: String, timeoutc: Int) {
    val code: String = codec
    val timeout: Int = timeoutc
  }

  private class LittleEndianDataInputStream(i: InputStream) extends DataInputStream(i) {
    def readLongLE(): Long = java.lang.Long.reverseBytes(super.readLong())

    def readIntLE(): Int = java.lang.Integer.reverseBytes(super.readInt())

    def readCharLE(): Char = java.lang.Character.reverseBytes(super.readChar())

    def readShortLE(): Short = java.lang.Short.reverseBytes(super.readShort())
  }

  private class LittleEndianDataOutputStream(i: OutputStream) extends DataOutputStream(i) {
    def writeLongLE(p: Long): Unit = super.writeLong(java.lang.Long.reverseBytes(p))

    def writeIntLE(p: Int): Unit = super.writeInt(java.lang.Integer.reverseBytes(p))

    def writeShortLE(p: Short): Unit = super.writeShort(java.lang.Short.reverseBytes(p))

    def writeCharLE(p: Char): Unit = super.writeChar(java.lang.Character.reverseBytes(p))
  }

}