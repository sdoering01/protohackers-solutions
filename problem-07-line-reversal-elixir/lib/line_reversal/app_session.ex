defmodule LineReversal.AppSession do
  use Task

  alias LineReversal.LRCP

  def serve(session, buf \\ "") do
    receive do
      {:lrcp, lrcp, data} ->
        buf = buf <> data
        {lines, buf} = get_full_lines(buf)

        for line <- lines do
          LRCP.send(lrcp, session, String.reverse(line) <> "\n")
        end

        serve(session, buf)

      {:lrcp_close, _lrcp, _session} ->
        nil
    end
  end

  def get_full_lines(buf) do
    [rest | lines] =
      buf
      |> String.split("\n")
      |> Enum.reverse()

    {Enum.reverse(lines), rest}
  end
end
