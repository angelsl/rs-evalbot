val scalaVer = "2.11.7"

name := "scalaeval"
version := "1.0"
scalaVersion := scalaVer
libraryDependencies += "org.scala-lang" % "scala-compiler" % scalaVer
mainClass in Compile := Some("org.github.angelsl.evalbot.scala.ScalaEval")
    