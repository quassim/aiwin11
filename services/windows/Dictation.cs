using System.Globalization;
using System.Speech.Recognition;
using Protocol = Aiwin11.V1;

/// Local dictation with the Windows desktop speech recognizer (SAPI, offline). Started only by
/// the user's click in the main window; listens to the default microphone for one phrase.
static class Dictation
{
    public static Protocol.DictationResult Listen(uint maxSeconds, long deadlineMs)
    {
        var result = new Protocol.DictationResult();
        var recognizers = SpeechRecognitionEngine.InstalledRecognizers();
        var info = recognizers.FirstOrDefault(r => r.Culture.TwoLetterISOLanguageName == CultureInfo.CurrentUICulture.TwoLetterISOLanguageName)
                   ?? recognizers.FirstOrDefault(r => r.Culture.TwoLetterISOLanguageName == "fr")
                   ?? recognizers.FirstOrDefault();
        if (info is null)
        {
            result.Status = Protocol.DictationResult.Types.Status.NoRecognizer;
            return result;
        }
        result.Language = info.Culture.Name;
        using var engine = new SpeechRecognitionEngine(info);
        engine.LoadGrammar(new DictationGrammar());
        try
        {
            engine.SetInputToDefaultAudioDevice();
        }
        catch (InvalidOperationException)
        {
            result.Status = Protocol.DictationResult.Types.Status.NoMicrophone;
            return result;
        }
        var remaining = Math.Max(0, deadlineMs - DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() - 500);
        var listen = TimeSpan.FromMilliseconds(Math.Min(remaining, Math.Min(maxSeconds, 10) * 1000));
        engine.InitialSilenceTimeout = listen;
        engine.BabbleTimeout = listen;
        var recognized = engine.Recognize(listen);
        var text = recognized?.Text?.Trim() ?? "";
        result.Text = text.Length <= 2000 ? text : text[..2000];
        result.Status = text.Length > 0 ? Protocol.DictationResult.Types.Status.Recognized : Protocol.DictationResult.Types.Status.Silence;
        return result;
    }
}
