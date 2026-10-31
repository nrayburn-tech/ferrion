import java.util.spi.ToolProvider;

/** The module. */
@Deprecated
module com.example.app {
  requires java.base;
  requires transitive com.example.api;
  requires static com.example.optional;
  requires static transitive com.example.both;
  requires transitive;
  requires transitive.name;
  exports com.example.app.api;
  exports com.example.app.internal to com.example.test, com.example.tool;
  opens com.example.app.model;
  opens com.example.app.resources to com.example.framework;
  uses ToolProvider;
  provides ToolProvider with com.example.app.Tool, com.example.app.OtherTool;
}
