/** Docs. */
@Deprecated
open module com.example.m {
  requires transitive static java.base;
  exports com.example.a to java.sql, java.xml;
  opens com.example.b;
  uses com.example.Spi;
  provides com.example.Spi with com.example.Impl, com.example.Impl2;
}
